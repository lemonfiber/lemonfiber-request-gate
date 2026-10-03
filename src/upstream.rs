//! The requests the gate builds, and how it sends them and hands back what came of them.

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use lemonfiber_sidecar::gate::Upstream;
use serde_json::Value;
use url::Url;

/// One request the gate built, whole: nothing in it came from the caller unread.
#[derive(Debug, Clone)]
pub(crate) struct Built {
    /// Its method.
    pub(crate) method: Method,
    /// Its path under the upstream's address.
    pub(crate) path: String,
    /// Its query parameters, each one the list names.
    pub(crate) query: Vec<(&'static str, String)>,
    /// Its headers: the gate's own credential, and nothing of the caller's.
    pub(crate) headers: HeaderMap,
    /// Its body, built from the fields the list names.
    pub(crate) body: Option<Value>,
}

impl Built {
    /// `method` on `path`, with no parameters, headers or body yet.
    pub(crate) fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            headers: HeaderMap::new(),
            body: None,
        }
    }

    /// This request with the header `name` set to `value`.
    pub(crate) fn header(mut self, name: header::HeaderName, value: HeaderValue) -> Self {
        self.headers.insert(name, value);
        self
    }

    /// This request with the query parameters `query`.
    pub(crate) fn query(mut self, query: Vec<(&'static str, String)>) -> Self {
        self.query = query;
        self
    }

    /// This request with `body`.
    pub(crate) fn body(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }
}

/// What the gate does with a call on the list.
#[derive(Debug)]
pub(crate) enum Plan {
    /// Send this request, and hand back what the upstream answers.
    Forward(Built),
    /// Record this removal, then send it and hand back what the upstream answers.
    Remove(Built),
    /// Answer with this, sending nothing further.
    Answer(Response),
}

/// Why the gate stopped short of sending a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// The call is not on the list, or does not hold to what the list checks.
    Refused,
    /// The upstream answered a read the gate made for its checks with this status.
    Upstream(StatusCode),
    /// The upstream did not answer, or answered something the gate cannot read.
    Unreachable,
}

/// One route's upstream, and the client every call to it is made with.
pub(crate) struct Reach<'a> {
    /// The client.
    pub(crate) client: &'a reqwest::Client,
    /// The route, as the core wrote it.
    pub(crate) upstream: &'a Upstream,
}

impl Reach<'_> {
    /// Send `built`, and say what the upstream answered.
    pub(crate) async fn send(&self, built: Built) -> Result<reqwest::Response, Stop> {
        let mut url = Url::parse(&format!(
            "{}{}",
            self.upstream.address.trim_end_matches('/'),
            built.path
        ))
        .map_err(|_| Stop::Unreachable)?;
        if !built.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&built.query);
        }
        let mut request = self
            .client
            .request(built.method, url)
            .headers(built.headers);
        if let Some(body) = &built.body {
            request = request.json(body);
        }
        request.send().await.map_err(|_| Stop::Unreachable)
    }

    /// Send `built`, a read the gate makes for its own checks, and read its answer.
    pub(crate) async fn read(&self, built: Built) -> Result<Value, Stop> {
        let answer = self.send(built).await?;
        if !answer.status().is_success() {
            return Err(Stop::Upstream(answer.status()));
        }
        answer.json().await.map_err(|_| Stop::Unreachable)
    }
}

/// The headers of an upstream's answer the gate hands back: what the body is, and
/// what a cache needs to tell whether it changed.
const PASSED: [header::HeaderName; 3] = [header::CONTENT_TYPE, header::ETAG, header::LAST_MODIFIED];

/// What the upstream answered, handed back with its status, its body and the headers
/// in [`PASSED`].
pub(crate) fn passed(answer: reqwest::Response) -> Response {
    let status = answer.status();
    let headers = kept(answer.headers());
    let mut response = Response::new(Body::from_stream(answer.bytes_stream()));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

/// The headers in [`PASSED`] that `headers` carries.
pub(crate) fn kept(headers: &HeaderMap) -> HeaderMap {
    let mut kept = HeaderMap::new();
    for name in PASSED {
        if let Some(value) = headers.get(&name) {
            kept.insert(name, value.clone());
        }
    }
    kept
}

#[cfg(test)]
mod tests;
