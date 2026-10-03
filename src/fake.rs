//! What the tests stand the gate up against: a configuration directory of its own, an
//! upstream that answers what it is told and remembers what it was sent, and a way to
//! ask the gate a call.

use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use lemonfiber_sidecar::gate::{Accepted, Credential, File, Kind, Tokens, Upstream, Upstreams};
use lemonfiber_sidecar::TokenHash;
use serde_json::Value;
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::serving::{routes, Service};
use crate::settings::Settings;

/// The token the Sonarr route accepts.
pub(crate) const TOKEN: &str = "the-sonarr-token";

/// The token the Radarr route accepts.
pub(crate) const RADARR_TOKEN: &str = "the-radarr-token";

/// The token the Jellyfin route accepts.
pub(crate) const JELLYFIN_TOKEN: &str = "the-jellyfin-token";

/// The key the gate holds for each upstream.
pub(crate) const KEY: &str = "the-upstream-key";

/// A configuration directory of its own, removed when dropped.
pub(crate) struct Config(PathBuf);

impl Config {
    pub(crate) fn new(name: &str) -> Self {
        static MADE: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "request-gate-{name}-{}-{}",
            std::process::id(),
            MADE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.0.clone()
    }

    pub(crate) fn write(&self, file: File, text: &str) {
        let _ = std::fs::write(self.0.join(file.name()), text);
    }

    pub(crate) fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    /// This directory with a Sonarr, a Radarr and a Jellyfin route, all reaching
    /// `address`, and each route's token.
    pub(crate) fn with_routes(self, address: &str) -> Self {
        let route = |route: &str, kind| Upstream {
            route: route.to_owned(),
            kind,
            address: address.to_owned(),
            credential: Credential::new(KEY),
        };
        let upstreams = Upstreams::of(vec![
            route("sonarr", Kind::Sonarr),
            route("radarr", Kind::Radarr),
            route("jellyfin", Kind::Jellyfin),
        ]);
        self.write(File::Upstreams, &upstreams.written());
        let accepted = |route: &str, token| Accepted {
            route: route.to_owned(),
            tokens: vec![TokenHash::of(token)],
        };
        let tokens = Tokens::of(vec![
            accepted("sonarr", TOKEN),
            accepted("radarr", RADARR_TOKEN),
            accepted("jellyfin", JELLYFIN_TOKEN),
        ]);
        self.write(File::Tokens, &tokens.written());
        self
    }

    /// The gate over this directory.
    pub(crate) fn service(&self) -> Arc<Service> {
        Service::new(
            Settings {
                config: self.path(),
            },
            crate::serving::client().unwrap_or_default(),
        )
    }

    /// Each entry the record holds, as `METHOD path`.
    pub(crate) fn recorded(&self) -> Vec<String> {
        lemonfiber_sidecar::gate::Record::read(&self.read(File::Record))
            .unwrap_or_default()
            .entries
            .iter()
            .map(|entry| format!("{} {}", entry.method, entry.path))
            .collect()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One request an upstream was sent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Option<Value>,
}

impl Seen {
    /// The header `name`, as text.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

/// What an upstream answers one method and path with.
#[derive(Debug, Clone)]
pub(crate) struct Answer {
    method: Method,
    path: &'static str,
    status: StatusCode,
    headers: Vec<(&'static str, String)>,
    body: Bytes,
}

/// `method` on `path` answered `200` with `body`.
pub(crate) fn json(method: Method, path: &'static str, body: &Value) -> Answer {
    Answer {
        method,
        path,
        status: StatusCode::OK,
        headers: vec![("content-type", "application/json".to_owned())],
        body: Bytes::from(body.to_string()),
    }
}

/// `method` on `path` answered with `status` and no body.
pub(crate) fn status(method: Method, path: &'static str, status: StatusCode) -> Answer {
    Answer {
        method,
        path,
        status,
        headers: Vec::new(),
        body: Bytes::new(),
    }
}

/// `method` on `path` answered `302`, sending the caller to `location`.
pub(crate) fn redirect(method: Method, path: &'static str, location: String) -> Answer {
    Answer {
        method,
        path,
        status: StatusCode::FOUND,
        headers: vec![("location", location)],
        body: Bytes::new(),
    }
}

/// `method` on `path` answered `200` with `body` as it is, and `headers`.
pub(crate) fn raw(
    method: Method,
    path: &'static str,
    headers: Vec<(&'static str, &'static str)>,
    body: &'static str,
) -> Answer {
    Answer {
        method,
        path,
        status: StatusCode::OK,
        headers: headers
            .into_iter()
            .map(|(name, value)| (name, value.to_owned()))
            .collect(),
        body: Bytes::from_static(body.as_bytes()),
    }
}

/// An upstream standing, and what it was sent.
#[derive(Clone)]
pub(crate) struct Fake {
    pub(crate) address: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    /// Everything it was sent, in order.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }

    /// What it was sent with `method` on `path`.
    pub(crate) fn sent(&self, method: &str, path: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|seen| seen.method == method && seen.path == path)
            .collect()
    }
}

/// An upstream answering `answers`, and `404` to anything else.
pub(crate) async fn upstream(answers: Vec<Answer>) -> Fake {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = (Arc::new(answers), Arc::clone(&seen));
    let app = Router::new().fallback(answered).with_state(state);
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await;
    let at = listener
        .as_ref()
        .ok()
        .and_then(|listener| listener.local_addr().ok())
        .map(|at| at.to_string())
        .unwrap_or_default();
    drop(listener.map(|listener| tokio::spawn(axum::serve(listener, app).into_future())));
    Fake {
        address: format!("http://{at}"),
        seen,
    }
}

type Told = (Arc<Vec<Answer>>, Arc<Mutex<Vec<Seen>>>);

async fn answered(State((answers, seen)): State<Told>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, usize::MAX).await.unwrap_or_default();
    if let Ok(mut seen) = seen.lock() {
        seen.push(Seen {
            method: parts.method.to_string(),
            path: parts.uri.path().to_owned(),
            query: parts.uri.query().map(str::to_owned),
            headers: parts.headers.clone(),
            body: serde_json::from_slice(&body).ok(),
        });
    }
    let Some(answer) = answers
        .iter()
        .find(|answer| answer.method == parts.method && answer.path == parts.uri.path())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = Response::new(Body::from(answer.body.clone()));
    *response.status_mut() = answer.status;
    for (name, value) in &answer.headers {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(value),
        ) {
            response.headers_mut().insert(name, value);
        }
    }
    response
}

/// What the gate answered one call.
#[derive(Debug)]
pub(crate) struct Answered {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: String,
}

impl Answered {
    /// The body, read as JSON.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_default()
    }
}

/// Ask `service` `method` on `uri`, with `headers` and `body`.
pub(crate) async fn ask(
    service: Arc<Service>,
    method: Method,
    uri: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Answered {
    let mut request = axum::http::Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request.body(body).unwrap_or_default();
    let Ok(response) = routes(service).oneshot(request).await;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    Answered {
        status,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    }
}

/// A JSON body.
pub(crate) fn body(value: &Value) -> Body {
    Body::from(value.to_string())
}
