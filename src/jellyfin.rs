//! The calls the gate answers on the Jellyfin route, and how it builds each.
//!
//! Sign-in calls answer to the member's own credential and carry no token; the gate
//! sends them under the request service's device, and ends any administrator's session
//! they open before the answer goes back. Library reads and the end of a session go
//! upstream under the gate's own key. The two calls that mint the request service a key
//! the gate answers itself.

use std::fmt::Write;

use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};

use crate::asked::{Asked, SCHEME};
use crate::shape::{fields, object, parameters, Shape};
use crate::upstream::{kept, passed, Built, Plan, Reach, Stop};

/// The name the request service gives itself to the media server: as the client its
/// sign-ins come from, and as the app it mints a key for.
const SEERR: &str = "Seerr";

/// The device the request service signs its owner in on. Each member's device is this,
/// an underscore and the member's name, in base64.
const DEVICE: &str = "BOT_seerr";

/// How the gate names itself to the media server under its own key.
const GATE: &str = env!("CARGO_PKG_NAME");

/// The version the gate reports to the media server.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The authorisation header field the request service sends its token in.
pub(crate) const TOKEN: &str = "Token";

/// The authorisation header field naming the device a call comes from.
const DEVICE_ID: &str = "DeviceId";

/// The field of a sign-in's answer, and of a key, that holds the token itself.
const ACCESS_TOKEN: &str = "AccessToken";

/// How many random bytes make a value that opens nothing.
const OPENS_NOTHING: usize = 32;

/// One call on the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Call {
    /// `GET /System/Info/Public`.
    PublicInfo,
    /// `POST /Users/AuthenticateByName`.
    SignIn,
    /// `POST /QuickConnect/Initiate`.
    QuickConnect,
    /// `GET /QuickConnect/Connect`.
    QuickConnected,
    /// `POST /Users/AuthenticateWithQuickConnect`.
    QuickConnectSignIn,
    /// `GET` or `HEAD /UserImage`.
    Avatar,
    /// `GET /System/Info`.
    Info,
    /// `GET /Users`.
    Users,
    /// `GET /Users/{id}`.
    User(String),
    /// `GET /Users/{id}/Views`.
    Views(String),
    /// `GET /Library/MediaFolders`.
    MediaFolders,
    /// `GET /Items`.
    Items,
    /// `GET /Items/Latest`.
    Latest,
    /// `GET /Shows/{id}/Seasons`.
    Seasons(String),
    /// `GET /Shows/{id}/Episodes`.
    Episodes(String),
    /// `DELETE /Devices`.
    EndSession,
    /// `POST /Auth/Keys`, answered by the gate.
    MintKey,
    /// `GET /Auth/Keys`, answered by the gate.
    Keys,
}

/// The call `asked` makes on the Jellyfin route, where it is one on the list.
pub(crate) fn listed(asked: &Asked) -> Option<Call> {
    let segments = asked.segments();
    let call = match (asked.method.clone(), segments.as_slice()) {
        (Method::GET, ["system", "info", "public"]) => Call::PublicInfo,
        (Method::POST, ["users", "authenticatebyname"]) => Call::SignIn,
        (Method::POST, ["quickconnect", "initiate"]) => Call::QuickConnect,
        (Method::GET, ["quickconnect", "connect"]) => Call::QuickConnected,
        (Method::POST, ["users", "authenticatewithquickconnect"]) => Call::QuickConnectSignIn,
        (Method::GET | Method::HEAD, ["userimage"]) => Call::Avatar,
        (Method::GET, ["system", "info"]) => Call::Info,
        (Method::GET, ["users"]) => Call::Users,
        (Method::GET, ["users", id]) => Call::User(identifier(id)?),
        (Method::GET, ["users", id, "views"]) => Call::Views(identifier(id)?),
        (Method::GET, ["library", "mediafolders"]) => Call::MediaFolders,
        (Method::GET, ["items"]) => Call::Items,
        (Method::GET, ["items", "latest"]) => Call::Latest,
        (Method::GET, ["shows", id, "seasons"]) => Call::Seasons(identifier(id)?),
        (Method::GET, ["shows", id, "episodes"]) => Call::Episodes(identifier(id)?),
        (Method::DELETE, ["devices"]) => Call::EndSession,
        (Method::POST, ["auth", "keys"]) => Call::MintKey,
        (Method::GET, ["auth", "keys"]) => Call::Keys,
        _ => return None,
    };
    Some(call)
}

/// `segment` as a media server identifier: letters, digits and hyphens only, so it can
/// name nothing but one item.
fn identifier(segment: &str) -> Option<String> {
    (!segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then(|| segment.to_owned())
}

/// The parameters a library listing carries.
const ITEMS: &[(&str, Shape)] = &[
    ("SortBy", Shape::Text),
    ("SortOrder", Shape::Text),
    ("IncludeItemTypes", Shape::Text),
    ("Recursive", Shape::Text),
    ("StartIndex", Shape::Text),
    ("ParentId", Shape::Text),
    ("collapseBoxSetItems", Shape::Text),
    ("ids", Shape::Text),
    ("fields", Shape::Text),
];

/// The parameters a listing of what was added lately carries.
const LATEST: &[(&str, Shape)] = &[
    ("Limit", Shape::Text),
    ("ParentId", Shape::Text),
    ("userId", Shape::Text),
];

/// The parameters a listing of a season's episodes carries.
const EPISODES: &[(&str, Shape)] = &[("seasonId", Shape::Text), ("fields", Shape::Text)];

impl Call {
    /// Whether the call must carry the route's token. Sign-ins answer to the member's
    /// own credential or to nobody's, and the gate answers the key mint itself.
    pub(crate) fn needs_token(&self) -> bool {
        !matches!(
            self,
            Self::PublicInfo
                | Self::SignIn
                | Self::QuickConnect
                | Self::QuickConnected
                | Self::QuickConnectSignIn
                | Self::Avatar
                | Self::MintKey
                | Self::Keys
        )
    }

    /// What the gate does with this call, made on `reach` by `asked`.
    pub(crate) async fn plan(self, reach: &Reach<'_>, asked: &Asked) -> Result<Plan, Stop> {
        let query = &asked.query;
        let plan = match self {
            Self::PublicInfo => Plan::Forward(public_info()),
            Self::Avatar => {
                Plan::Forward(Built::new(asked.method.clone(), "/UserImage").query(avatar(asked)?))
            }
            Self::QuickConnect => Plan::Forward(
                Built::new(Method::POST, "/QuickConnect/Initiate")
                    .header(header::AUTHORIZATION, device(asked, None)?),
            ),
            Self::QuickConnected => Plan::Forward(
                Built::new(Method::GET, "/QuickConnect/Connect")
                    .header(header::AUTHORIZATION, device(asked, None)?)
                    .query(parameters(query, &[("secret", Shape::Text)])?),
            ),
            Self::SignIn => {
                let body = fields(
                    &object(asked)?,
                    &[("Username", Shape::Text), ("Pw", Shape::Text)],
                )?;
                let built =
                    Built::new(Method::POST, "/Users/AuthenticateByName").body(Value::Object(body));
                Plan::Answer(signed_in(reach, asked, built).await?)
            }
            Self::QuickConnectSignIn => {
                let body = fields(&object(asked)?, &[("Secret", Shape::Text)])?;
                let built = Built::new(Method::POST, "/Users/AuthenticateWithQuickConnect")
                    .body(Value::Object(body));
                Plan::Answer(signed_in(reach, asked, built).await?)
            }
            Self::Info => Plan::Forward(keyed(reach, Method::GET, "/System/Info")?),
            Self::Users => Plan::Forward(keyed(reach, Method::GET, "/Users")?),
            Self::User(id) => Plan::Forward(keyed(reach, Method::GET, &format!("/Users/{id}"))?),
            Self::Views(id) => {
                Plan::Forward(keyed(reach, Method::GET, &format!("/Users/{id}/Views"))?)
            }
            Self::MediaFolders => {
                Plan::Forward(keyed(reach, Method::GET, "/Library/MediaFolders")?)
            }
            Self::Items => {
                Plan::Forward(keyed(reach, Method::GET, "/Items")?.query(parameters(query, ITEMS)?))
            }
            Self::Latest => Plan::Forward(
                keyed(reach, Method::GET, "/Items/Latest")?.query(parameters(query, LATEST)?),
            ),
            Self::Seasons(id) => {
                Plan::Forward(keyed(reach, Method::GET, &format!("/Shows/{id}/Seasons"))?)
            }
            Self::Episodes(id) => Plan::Forward(
                keyed(reach, Method::GET, &format!("/Shows/{id}/Episodes"))?
                    .query(parameters(query, EPISODES)?),
            ),
            Self::EndSession => Plan::Forward(
                keyed(reach, Method::DELETE, "/Devices")?
                    .query(vec![("Id", seerrs(asked.query.one("Id").ok().flatten())?)]),
            ),
            Self::MintKey => {
                if asked.query.one("App") != Ok(Some(SEERR)) {
                    return Err(Stop::Refused);
                }
                Plan::Answer(StatusCode::NO_CONTENT.into_response())
            }
            Self::Keys => Plan::Answer(
                Json(json!({
                    "Items": [{ "AppName": SEERR, ACCESS_TOKEN: opens_nothing()? }],
                    "TotalRecordCount": 1,
                    "StartIndex": 0,
                }))
                .into_response(),
            ),
        };
        Ok(plan)
    }
}

/// `GET /System/Info/Public`, with no credential: the server's name and version, which
/// the gate also asks to learn whether it forwards to that version.
pub(crate) fn public_info() -> Built {
    Built::new(Method::GET, "/System/Info/Public")
}

/// `method` on `path`, under the gate's own key.
fn keyed(reach: &Reach<'_>, method: Method, path: &str) -> Result<Built, Stop> {
    let key = authorisation(GATE, GATE, Some(reach.upstream.credential.reveal()))?;
    Ok(Built::new(method, path).header(header::AUTHORIZATION, key))
}

/// The authorisation a sign-in carries: the request service's device, which must be one
/// it assigns, and `token` where the call ends that session.
fn device(asked: &Asked, token: Option<&str>) -> Result<HeaderValue, Stop> {
    authorisation(SEERR, &seerrs(asked.authorisation.get(DEVICE_ID))?, token)
}

/// A `MediaBrowser` authorisation from the client `client` on the device `device`,
/// carrying `token` where there is one.
fn authorisation(client: &str, device: &str, token: Option<&str>) -> Result<HeaderValue, Stop> {
    let token = token
        .map(|token| format!(", {TOKEN}=\"{token}\""))
        .unwrap_or_default();
    HeaderValue::from_str(&format!(
        "{SCHEME}Client=\"{client}\", Device=\"{client}\", {DEVICE_ID}=\"{device}\", Version=\"{VERSION}\"{token}"
    ))
    .map_err(|_| Stop::Unreachable)
}

/// `sent`, where it names a device the request service assigns: its owner's, or a
/// member's, as base64.
fn seerrs(sent: Option<&str>) -> Result<String, Stop> {
    let id = sent.ok_or(Stop::Refused)?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(id)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default();
    let member = decoded
        .strip_prefix(DEVICE)
        .and_then(|rest| rest.strip_prefix('_'))
        .is_some_and(|name| !name.is_empty());
    if id == DEVICE || decoded == DEVICE || member {
        Ok(id.to_owned())
    } else {
        Err(Stop::Refused)
    }
}

/// The one parameter an avatar carries: the account it belongs to.
fn avatar(asked: &Asked) -> Result<Vec<(&'static str, String)>, Stop> {
    match asked.query.one("UserId").map_err(|_| Stop::Refused)? {
        Some(id) => Ok(vec![("UserId", identifier(id).ok_or(Stop::Refused)?)]),
        None => Ok(Vec::new()),
    }
}

/// Send the sign-in `built`, and answer with what came of it, with no administrator's
/// session in it.
///
/// A member's answer goes back as it came. An administrator's session is ended, and the
/// answer goes back with its token replaced by a value that opens nothing; an answer
/// that cannot be read as either does not go back at all.
async fn signed_in(reach: &Reach<'_>, asked: &Asked, built: Built) -> Result<Response, Stop> {
    let built = built.header(header::AUTHORIZATION, device(asked, None)?);
    let answer = reach.send(built).await?;
    if !answer.status().is_success() {
        return Ok(passed(answer));
    }
    let status = answer.status();
    let headers = kept(answer.headers());
    let body = answer.bytes().await.map_err(|_| Stop::Unreachable)?;
    let mut read: Value = serde_json::from_slice(&body).map_err(|_| Stop::Unreachable)?;
    let administrator = read
        .pointer("/User/Policy/IsAdministrator")
        .and_then(Value::as_bool)
        .ok_or(Stop::Unreachable)?;
    if !administrator {
        return Ok((status, headers, body).into_response());
    }

    let token = read
        .get(ACCESS_TOKEN)
        .and_then(Value::as_str)
        .ok_or(Stop::Unreachable)?;
    let logout = Built::new(Method::POST, "/Sessions/Logout")
        .header(header::AUTHORIZATION, device(asked, Some(token))?);
    if !reach.send(logout).await?.status().is_success() {
        return Err(Stop::Unreachable);
    }
    let placeholder = Value::from(opens_nothing()?);
    if let Some(token) = read.get_mut(ACCESS_TOKEN) {
        *token = placeholder;
    }
    Ok((status, Json(read)).into_response())
}

/// A random value, in hexadecimal, that no route accepts and the media server never
/// issued.
fn opens_nothing() -> Result<String, Stop> {
    let mut bytes = [0u8; OPENS_NOTHING];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Stop::Unreachable)?;
    Ok(bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    }))
}

#[cfg(test)]
mod tests;
