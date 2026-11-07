//! What the tests stand the gate up against: a configuration directory of its own, an
//! upstream that answers what it is told and remembers what it was sent, and a way to
//! ask the gate a call.

use std::future::IntoFuture;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::formats::gate::{Accepted, Credential, File, Kind, Tokens, Upstream, Upstreams};
use crate::formats::{Pins, TokenHash};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use serde_json::Value;
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::serving::{routes, Service};
use crate::settings::Settings;
use lemonfiber_ports::service::PlexLink;

/// The token the series route accepts.
pub(crate) const TV_TOKEN: &str = "the-tv-token";

/// The token the film route accepts.
pub(crate) const MOVIES_TOKEN: &str = "the-movies-token";

/// The token the media server's route accepts, Jellyfin's or Emby's.
pub(crate) const JELLYFIN_TOKEN: &str = "the-jellyfin-token";

/// The series route's name.
pub(crate) const TV_ROUTE: &str = "tv";

/// The film route's name.
pub(crate) const MOVIES_ROUTE: &str = "movies";

/// The Jellyfin route's name.
pub(crate) const JELLYFIN_ROUTE: &str = "jellyfin";

/// The Emby route's name.
pub(crate) const EMBY_ROUTE: &str = "emby";

/// The Plex route's name.
pub(crate) const PLEX_ROUTE: &str = "plex";

/// The token the Plex route admits in the tests that give it one.
pub(crate) const PLEX_TOKEN: &str = "the-plex-token";

/// The key the gate holds for each upstream.
pub(crate) const KEY: &str = "the-upstream-key";

/// The media server major every test route supports.
pub(crate) const MAJOR: u32 = 10;

/// The version an upstream reports unless a test says otherwise.
pub(crate) const VERSION: &str = "10.11.11";

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

    /// This directory with a series, a film and a Jellyfin route, all reaching `address`,
    /// the Jellyfin one pinning a certificate nothing presents, and each route's token.
    pub(crate) fn with_routes(self, address: &str) -> Self {
        self.routed(
            address,
            address,
            &lemonfiber_pinned::sha256(b"no certificate"),
        )
    }

    /// This directory with a series and a film route reaching `fake` and a Jellyfin one
    /// reaching it over TLS, pinning the certificate it presents, and each route's token.
    pub(crate) fn with_upstream(self, fake: &Fake) -> Self {
        self.routed(&fake.address, &fake.tls, &fake.pin)
    }

    fn routed(self, address: &str, encrypted: &str, pin: &str) -> Self {
        self.served(address, encrypted, pin, (JELLYFIN_ROUTE, Kind::Jellyfin))
    }

    /// This directory with a series and a film route reaching `fake` and an Emby one
    /// reaching it over TLS, pinning the certificate it presents, and each route's token.
    pub(crate) fn with_emby(self, fake: &Fake) -> Self {
        self.served(
            &fake.address,
            &fake.tls,
            &fake.pin,
            (EMBY_ROUTE, Kind::Emby),
        )
    }

    fn served(self, address: &str, encrypted: &str, pin: &str, server: (&str, Kind)) -> Self {
        let route = |route: &str, kind: Kind| {
            let media_server = kind.media().is_none();
            Upstream {
                route: route.to_owned(),
                kind,
                address: if media_server {
                    encrypted.to_owned()
                } else {
                    address.to_owned()
                },
                credential: Credential::new(KEY),
                majors: if media_server {
                    vec![MAJOR]
                } else {
                    Vec::new()
                },
                pins: if media_server {
                    Pins::of(&[pin.to_owned()]).unwrap_or_default()
                } else {
                    Pins::default()
                },
                link: None,
            }
        };
        let upstreams = Upstreams::of(vec![
            route(TV_ROUTE, Kind::Tv),
            route(MOVIES_ROUTE, Kind::Movies),
            route(server.0, server.1),
        ]);
        self.write(File::Upstreams, &upstreams.written());
        let accepted = |route: &str, token| Accepted {
            route: route.to_owned(),
            tokens: vec![TokenHash::of(token)],
        };
        let tokens = Tokens::of(vec![
            accepted(TV_ROUTE, TV_TOKEN),
            accepted(MOVIES_ROUTE, MOVIES_TOKEN),
            accepted(server.0, JELLYFIN_TOKEN),
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

    /// The gate over this directory, reaching plex.tv at `plex_tv`.
    pub(crate) fn reaching(&self, plex_tv: &str) -> Arc<Service> {
        Service::reaching(
            Settings {
                config: self.path(),
            },
            crate::serving::client().unwrap_or_default(),
            plex_tv,
        )
    }

    /// This directory with one Plex route reaching `fake` over TLS, pinning the certificate
    /// it presents, linked as `link`, admitting `admits` where it is a token, and
    /// supporting `majors`.
    pub(crate) fn with_plex(
        self,
        fake: &Fake,
        link: PlexLink,
        admits: Option<&str>,
        majors: Vec<u32>,
    ) -> Self {
        let upstreams = Upstreams::of(vec![Upstream {
            route: PLEX_ROUTE.to_owned(),
            kind: Kind::Plex,
            address: fake.tls.clone(),
            credential: Credential::new(KEY),
            majors,
            pins: Pins::of(std::slice::from_ref(&fake.pin)).unwrap_or_default(),
            link: Some(link),
        }]);
        self.write(File::Upstreams, &upstreams.written());
        let tokens = Tokens::of(vec![Accepted {
            route: PLEX_ROUTE.to_owned(),
            tokens: admits.map(TokenHash::of).into_iter().collect(),
        }]);
        self.write(File::Tokens, &tokens.written());
        self
    }

    /// Each entry the record holds, as `METHOD path`.
    pub(crate) fn recorded(&self) -> Vec<String> {
        crate::formats::gate::Record::read(&self.read(File::Record))
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

/// `operation` of `library.curate` answered with `answer`.
pub(crate) fn curated(operation: &str, answer: &Value) -> Answer {
    json(Method::POST, curate(operation), answer)
}

/// `operation` of `library.curate` refused as `kind`, as an adapter refuses it.
pub(crate) fn curate_refused(operation: &str, kind: lemonfiber_contract::wire::Kind) -> Answer {
    let refusal = lemonfiber_contract::Refusal::new(kind, "refused");
    Answer {
        method: Method::POST,
        path: curate(operation),
        status: StatusCode::from_u16(kind.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        headers: vec![(
            "content-type",
            lemonfiber_contract::wire::PROBLEM.to_owned(),
        )],
        body: Bytes::from(serde_json::to_string(&refusal).unwrap_or_default()),
    }
}

/// The path `operation` of `library.curate` is asked at.
pub(crate) fn curate(operation: &str) -> &'static str {
    use lemonfiber_contract::capabilities::library::curate;
    let path = lemonfiber_contract::path(curate::CAPABILITY, curate::MAJOR, operation);
    Box::leak(path.into_boxed_str())
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

/// The path the gate asks the media server's version on.
const PROBE: &str = "/System/Info/Public";

/// An upstream standing, over plain HTTP at `address` and over TLS at `tls` presenting
/// the certificate `pin` is the SHA-256 of, and what it was sent.
#[derive(Clone)]
pub(crate) struct Fake {
    pub(crate) address: String,
    pub(crate) tls: String,
    pub(crate) pin: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    /// Everything it was sent, in order.
    pub(crate) fn all(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }

    /// Everything it was sent but the gate asking its version, in order.
    pub(crate) fn seen(&self) -> Vec<Seen> {
        self.all()
            .into_iter()
            .filter(|seen| !(seen.method == "GET" && seen.path == PROBE))
            .collect()
    }

    /// What it was sent with `method` on `path`, the gate asking its version included.
    pub(crate) fn sent(&self, method: &str, path: &str) -> Vec<Seen> {
        self.all()
            .into_iter()
            .filter(|seen| seen.method == method && seen.path == path)
            .collect()
    }
}

/// An upstream answering `answers`; then, where they do not say, its version as a
/// supported media server; and `404` to anything else.
pub(crate) async fn upstream(mut answers: Vec<Answer>) -> Fake {
    answers.push(json(
        Method::GET,
        "/System/Info/Public",
        &serde_json::json!({ "Version": VERSION, "ServerName": "home" }),
    ));
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
    let encrypted = lemonfiber_pinned::fake::tls(app.clone());
    drop(listener.map(|listener| tokio::spawn(axum::serve(listener, app).into_future())));
    let (tls, pin) = encrypted.map_or_else(Default::default, |served| {
        let base = served
            .base
            .as_url()
            .as_str()
            .trim_end_matches('/')
            .to_owned();
        (base, served.pin)
    });
    Fake {
        address: format!("http://{at}"),
        tls,
        pin,
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
    ask_on(routes(service), method, uri, headers, body).await
}

/// Ask `router` `method` on `uri`, with `headers` and `body`.
pub(crate) async fn ask_on(
    router: Router,
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
    let Ok(response) = router.oneshot(request).await;
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
