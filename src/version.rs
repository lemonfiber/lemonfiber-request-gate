//! Whether a route's upstream runs a version the gate forwards to.
//!
//! An \*arr is supported when it answers its status on the one API the gate speaks.
//! The media server is supported when the first number of its version is one of the
//! majors its route names. Each route is asked on the first call that reaches it, and
//! what it answered holds until the core writes that route differently.

use std::collections::HashMap;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use lemonfiber_sidecar::gate::{Kind, Upstream};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::upstream::{Reach, Stop};
use crate::{arr, jellyfin};

/// What a route's upstream answered when asked its version.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// A version the gate forwards to.
    Supported,
    /// The media server's version, which is none of its route's majors.
    Unsupported(String),
}

/// What each route's upstream answered, with the route as it was when asked.
#[derive(Debug, Default)]
pub(crate) struct Versions {
    found: Mutex<HashMap<String, (Upstream, Verdict)>>,
}

impl Versions {
    /// Whether `reach`'s upstream runs a version the gate forwards to; where it does
    /// not, or cannot be asked, the answer the call gets instead.
    pub(crate) async fn supported(&self, reach: &Reach<'_>) -> Result<(), Response> {
        let upstream = reach.upstream;
        let known = self
            .found
            .lock()
            .await
            .get(&upstream.route)
            .filter(|(asked, _)| asked == upstream)
            .map(|(_, verdict)| verdict.clone());
        let verdict = if let Some(verdict) = known {
            verdict
        } else {
            let verdict = asked(reach).await.map_err(unanswered)?;
            self.found
                .lock()
                .await
                .insert(upstream.route.clone(), (upstream.clone(), verdict.clone()));
            verdict
        };
        match verdict {
            Verdict::Supported => Ok(()),
            Verdict::Unsupported(version) => Err(unsupported(&version)),
        }
    }
}

/// Ask `reach`'s upstream its version.
async fn asked(reach: &Reach<'_>) -> Result<Verdict, Stop> {
    match reach.upstream.kind {
        Kind::Sonarr | Kind::Radarr => {
            reach.read(arr::status(reach)?).await?;
            Ok(Verdict::Supported)
        }
        Kind::Jellyfin => {
            let info = reach.read(jellyfin::public_info()).await?;
            let version = info
                .get("Version")
                .and_then(Value::as_str)
                .ok_or(Stop::Unreachable)?;
            let major = version
                .split('.')
                .next()
                .and_then(|first| first.parse::<u32>().ok());
            Ok(
                if major.is_some_and(|major| reach.upstream.majors.contains(&major)) {
                    Verdict::Supported
                } else {
                    Verdict::Unsupported(version.to_owned())
                },
            )
        }
    }
}

/// The answer to a call whose upstream could not say its version: the status it
/// answered with, or `502` where it did not answer.
fn unanswered(stop: Stop) -> Response {
    match stop {
        Stop::Upstream(status) => status.into_response(),
        _ => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// The answer to every call on a route whose media server runs `version`.
fn unsupported(version: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )],
        format!("Jellyfin {version} is not a version this gate supports"),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
