//! What the request service sent, read into the parts the list is matched against.
//!
//! Nothing here is passed upstream. The list reads the method, the path's segments,
//! the query parameters it names and the body fields it names, and builds its own
//! request from them.

use axum::body::Bytes;
use axum::http::{Method, Uri};

/// One call, as the request service made it.
#[derive(Debug, Clone)]
pub(crate) struct Asked {
    /// Its method.
    pub(crate) method: Method,
    /// Its path, as sent, without the query string: what the record holds.
    pub(crate) path: String,
    /// The path's first segment, which names the route.
    pub(crate) route: String,
    /// The segments after the route, in lower case: the upstreams match a path
    /// without regard to case, and so does the list.
    pub(crate) segments: Vec<String>,
    /// Its query parameters.
    pub(crate) query: Query,
    /// Its body, where it was no larger than the gate reads.
    pub(crate) body: Option<Bytes>,
}

impl Asked {
    /// The call `method` made to `uri` with `body`.
    pub(crate) fn new(method: Method, uri: &Uri, body: Option<Bytes>) -> Self {
        let path = uri.path().to_owned();
        let mut segments = path.split('/').skip(1);
        let route = segments.next().unwrap_or_default().to_owned();
        Self {
            method,
            route,
            segments: segments.map(str::to_ascii_lowercase).collect(),
            query: Query::parse(uri.query()),
            body,
            path,
        }
    }

    /// The segments after the route, as the list matches them.
    pub(crate) fn segments(&self) -> Vec<&str> {
        self.segments.iter().map(String::as_str).collect()
    }
}

/// A call's query parameters, in the order sent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Query(Vec<(String, String)>);

/// A parameter was sent twice, so which one is meant cannot be told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Twice;

impl Query {
    /// The parameters in `query`.
    fn parse(query: Option<&str>) -> Self {
        Self(
            url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
                .into_owned()
                .collect(),
        )
    }

    /// The one value of the parameter `name`, matched without regard to case, as the
    /// upstreams match it; nothing where it was not sent.
    pub(crate) fn one(&self, name: &str) -> Result<Option<&str>, Twice> {
        let mut named = self
            .0
            .iter()
            .filter(|(sent, _)| sent.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str());
        let first = named.next();
        match named.next() {
            Some(_) => Err(Twice),
            None => Ok(first),
        }
    }
}

#[cfg(test)]
mod tests;
