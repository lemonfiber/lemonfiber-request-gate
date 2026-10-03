//! What the gate answers, and the health check its image runs.

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use lemonfiber_sidecar::gate::{Kind, Outcome};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::asked::Asked;
use crate::files::Files;
use crate::settings::Settings;
use crate::upstream::{passed, Plan, Reach, Stop};
use crate::version::Versions;
use crate::{arr, jellyfin};

/// The path the image's health check asks.
const HEALTH: &str = "/health";

/// The most of a call's body the gate reads: far more than any write on the list
/// carries, and a bound on what one call can make it hold.
const BODY: usize = 1024 * 1024;

/// What every call is answered with: the files, the client, and what each upstream
/// answered when asked its version.
pub(crate) struct Service {
    files: Files,
    client: reqwest::Client,
    versions: Versions,
}

impl Service {
    /// A service over `settings`, sending every call with `client`.
    pub(crate) fn new(settings: Settings, client: reqwest::Client) -> Arc<Self> {
        Arc::new(Self {
            files: Files::new(settings.config),
            client,
            versions: Versions::default(),
        })
    }
}

/// The client every call is sent with. It goes only where the gate sends it: it
/// follows no redirect and goes through no proxy the environment names.
pub(crate) fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
}

/// The routes the gate answers: its health, and every call, which is matched against
/// the list of the route it names.
pub(crate) fn routes(service: Arc<Service>) -> Router {
    Router::new().fallback(answer).with_state(service)
}

/// Serve [`routes`] on `at` until `stopped` resolves.
pub(crate) async fn serve(
    at: SocketAddr,
    service: Arc<Service>,
    stopped: impl std::future::Future<Output = ()> + Send + 'static,
) -> ExitCode {
    let listener = match TcpListener::bind(at).await {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("request-gate: could not listen on {at}: {error}");
            return ExitCode::FAILURE;
        }
    };
    axum::serve(listener, routes(service))
        .with_graceful_shutdown(stopped)
        .await
        .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
}

/// One call on a route, from the request service.
enum Listed {
    /// On a Sonarr or Radarr route.
    Arr(arr::Call),
    /// On the Jellyfin route.
    Jellyfin(jellyfin::Call),
}

/// The answer to one call.
///
/// A call not on its route's list is refused with `403`, sent nowhere, and recorded.
/// One on the list without the route's token is answered `401`. One on a route whose
/// upstream runs a version the gate does not forward to is answered `503`. Every other
/// is built by the gate and sent, and a removal is recorded before it is.
async fn answer(State(service): State<Arc<Service>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    if parts.method == Method::GET && parts.uri.path() == HEALTH {
        return StatusCode::OK.into_response();
    }
    let asked = Asked::new(
        parts.method,
        &parts.uri,
        &parts.headers,
        to_bytes(body, BODY).await.ok(),
    );

    let Some(upstreams) = service.files.upstreams().await else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Some(upstream) = upstreams.route(&asked.route) else {
        return refused(&service, &asked).await;
    };
    let listed = match upstream.kind {
        Kind::Sonarr | Kind::Radarr => arr::listed(upstream.kind, &asked).map(Listed::Arr),
        Kind::Jellyfin => jellyfin::listed(&asked).map(Listed::Jellyfin),
    };
    let Some(listed) = listed else {
        return refused(&service, &asked).await;
    };

    let token = match &listed {
        Listed::Arr(_) => Some(asked.query.one(arr::TOKEN).ok().flatten()),
        Listed::Jellyfin(call) => call
            .needs_token()
            .then(|| asked.authorisation.get(jellyfin::TOKEN)),
    };
    if let Some(token) = token {
        if !service.files.accepts(&upstream.route, token).await {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    let reach = Reach {
        client: &service.client,
        upstream,
    };
    if let Err(answer) = service.versions.supported(&reach).await {
        return answer;
    }
    let plan = match listed {
        Listed::Arr(call) => call.plan(&reach, &asked).await,
        Listed::Jellyfin(call) => call.plan(&reach, &asked).await,
    };
    match plan {
        Ok(Plan::Forward(built)) => sent(reach.send(built).await),
        Ok(Plan::Remove(built)) => {
            let recorded = service
                .files
                .record(
                    &asked.route,
                    asked.method.as_str(),
                    &asked.path,
                    Outcome::Removed,
                )
                .await;
            if !recorded {
                eprintln!("request-gate: a removal could not be recorded, so it was not passed on");
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            sent(reach.send(built).await)
        }
        Ok(Plan::Answer(response)) => response,
        Err(stop) => stopped(&service, &asked, stop).await,
    }
}

/// What the upstream answered a call the gate sent, or why it could not be sent.
fn sent(answer: Result<reqwest::Response, Stop>) -> Response {
    answer.map_or_else(|_| StatusCode::BAD_GATEWAY.into_response(), passed)
}

/// The answer to a call the gate stopped short of sending.
async fn stopped(service: &Service, asked: &Asked, stop: Stop) -> Response {
    match stop {
        Stop::Refused => refused(service, asked).await,
        Stop::Upstream(status) => status.into_response(),
        Stop::Unreachable => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Refuse `asked`, and record it.
async fn refused(service: &Service, asked: &Asked) -> Response {
    let recorded = service
        .files
        .record(
            &asked.route,
            asked.method.as_str(),
            &asked.path,
            Outcome::Refused,
        )
        .await;
    if !recorded {
        eprintln!("request-gate: a refused call could not be recorded");
    }
    StatusCode::FORBIDDEN.into_response()
}

/// Whether the service on this container's `port` answers its health.
///
/// The image is distroless, with no shell and no HTTP client, so its health check
/// is this binary asking itself over loopback.
pub(crate) async fn healthy(port: u16) -> ExitCode {
    if answers_ok(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Whether `at` answers `GET /health` with `200`.
async fn answers_ok(at: SocketAddr) -> bool {
    health_answer(at)
        .await
        .is_some_and(|answer| answer.starts_with(b"HTTP/1.1 200"))
}

/// What `at` answers to `GET /health`, where it answers at all.
async fn health_answer(at: SocketAddr) -> Option<Vec<u8>> {
    let mut stream = TcpStream::connect(at).await.ok()?;
    let asked = format!("GET {HEALTH} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    let written = stream.write_all(asked.as_bytes()).await;
    let mut answer = Vec::new();
    let read = stream.read_to_end(&mut answer).await;
    (written.is_ok() && read.is_ok()).then_some(answer)
}

#[cfg(test)]
mod tests;
