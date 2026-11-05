//! The calls the gate answers on the Plex route, and how it builds each.
//!
//! The request service reads Plex's libraries and items, and nothing else. Every read
//! goes upstream as the request service's own managed account, whatever token it was
//! asked with, so Plex is read as that account in every mode.

use axum::http::{header, HeaderName, HeaderValue, Method};

use crate::asked::Asked;
use crate::upstream::{Built, Plan, Reach, Stop};

/// The header Plex reads a token from, and the query parameter.
pub(crate) const TOKEN: &str = "X-Plex-Token";

/// [`TOKEN`] as the header name it is sent as.
const TOKEN_HEADER: &str = "x-plex-token";

/// The headers a library listing pages by.
const PAGING: [&str; 2] = ["x-plex-container-start", "x-plex-container-size"];

/// The parameters a library listing carries: `addedAt>>` is Plex's later-than filter.
const LISTING: [&str; 4] = ["includeGuids", "type", "sort", "addedAt>>"];

/// The parameter an item's read carries.
const CHILDREN: &str = "includeChildren";

/// The longest id Plex files a library or an item under.
const LONGEST_ID: usize = 20;

/// One call on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    /// `GET /`: which server it is.
    Status,
    /// `GET /library/sections`.
    Sections,
    /// `GET /library/sections/{id}/all`.
    Contents(String),
    /// `GET /library/metadata/{key}`.
    Metadata(String),
    /// `GET /library/metadata/{key}/children`.
    Children(String),
}

/// The call `asked` makes on the Plex route, where it is one on the list.
pub(crate) fn listed(asked: &Asked) -> Option<Call> {
    if asked.method != Method::GET {
        return None;
    }
    let segments = asked.segments();
    let call = match segments.as_slice() {
        [] | [""] => Call::Status,
        ["library", "sections"] => Call::Sections,
        ["library", "sections", id, "all"] => Call::Contents(an_id(id)?),
        ["library", "metadata", key] => Call::Metadata(an_id(key)?),
        ["library", "metadata", key, "children"] => Call::Children(an_id(key)?),
        _ => return None,
    };
    Some(call)
}

/// `segment` as an id Plex files a library or an item under: digits alone.
fn an_id(segment: &str) -> Option<String> {
    ((1..=LONGEST_ID).contains(&segment.len()) && segment.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| segment.to_owned())
}

/// The token `asked` presents: in the header Plex reads it from, or in the query, and the
/// same token where it is in both.
pub(crate) fn token(asked: &Asked) -> Option<&str> {
    let header = asked
        .headers
        .get(TOKEN)
        .and_then(|value| value.to_str().ok());
    let query = asked.query.one(TOKEN).ok()?;
    match (header, query) {
        (Some(header), Some(query)) if header != query => None,
        (Some(token), _) | (None, Some(token)) => Some(token),
        (None, None) => None,
    }
}

impl Call {
    /// What the gate does with this call, made on `reach` by `asked`.
    pub(crate) fn plan(self, reach: &Reach<'_>, asked: &Asked) -> Result<Plan, Stop> {
        let built = match self {
            Self::Status => keyed(reach, "/")?,
            Self::Sections => keyed(reach, "/library/sections")?,
            Self::Contents(id) => paged(
                keyed(reach, &format!("/library/sections/{id}/all"))?
                    .query(parameters(asked, &LISTING)?),
                asked,
            )?,
            Self::Metadata(key) => keyed(reach, &format!("/library/metadata/{key}"))?
                .query(parameters(asked, &[CHILDREN])?),
            Self::Children(key) => keyed(reach, &format!("/library/metadata/{key}/children"))?,
        };
        Ok(Plan::Forward(built))
    }
}

/// `GET /` under the route's own credential: which server it is and what version it runs.
pub(crate) fn status(reach: &Reach<'_>) -> Result<Built, Stop> {
    keyed(reach, "/")
}

/// `GET path`, asked as the request service's own account and read as JSON.
fn keyed(reach: &Reach<'_>, path: &str) -> Result<Built, Stop> {
    let token =
        HeaderValue::from_str(reach.upstream.credential.reveal()).map_err(|_| Stop::Unreachable)?;
    Ok(Built::new(Method::GET, path)
        .header(HeaderName::from_static(TOKEN_HEADER), token)
        .header(header::ACCEPT, HeaderValue::from_static("application/json")))
}

/// The parameters of `names` that `asked` sends, each once and in the characters a
/// listing's parameters are written in.
fn parameters(asked: &Asked, names: &[&'static str]) -> Result<Vec<(&'static str, String)>, Stop> {
    let mut kept = Vec::new();
    for name in names {
        if let Some(value) = asked.query.one(name).map_err(|_| Stop::Refused)? {
            if !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b":,.-_".contains(&byte))
            {
                return Err(Stop::Refused);
            }
            kept.push((*name, value.to_owned()));
        }
    }
    Ok(kept)
}

/// `built`, paged as `asked` pages it, by numbers alone.
fn paged(mut built: Built, asked: &Asked) -> Result<Built, Stop> {
    for name in PAGING {
        if let Some(value) = asked.headers.get(name) {
            let number = value
                .to_str()
                .ok()
                .filter(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
                .ok_or(Stop::Refused)?;
            built = built.header(
                HeaderName::from_static(name),
                HeaderValue::from_str(number).map_err(|_| Stop::Refused)?,
            );
        }
    }
    Ok(built)
}

#[cfg(test)]
mod tests;
