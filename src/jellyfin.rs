//! The calls the gate answers on the Jellyfin route and the Emby route, and how it builds
//! each. Emby is answered on the same list as Jellyfin, each call in Emby's own API: its
//! paths, and its own headers for the device a call comes from and the token it carries.
//! Emby has no Quick Connect, so those calls are refused on its route.
//!
//! Sign-in calls answer to the member's own credential and carry no token; the gate
//! sends them under the request service's device, and ends any administrator's session
//! they open before the answer goes back. Library reads and the end of a session go
//! upstream under the gate's own key. The two calls that mint the request service a key
//! the gate answers itself.

use std::fmt::Write;

use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Value};

use crate::asked::{Asked, SCHEME};
use crate::formats::gate::Kind;
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

/// The header Emby reads the client and the device a call comes from in.
const EMBY_AUTHORISATION: HeaderName = HeaderName::from_static("x-emby-authorization");

/// How the scheme of Emby's authorisation header is spelled.
const EMBY_SCHEME: &str = "Emby ";

/// The header Emby reads a call's token from.
const EMBY_TOKEN: HeaderName = HeaderName::from_static("x-emby-token");

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
    /// `POST /QuickConnect/Initiate`, on Jellyfin.
    QuickConnect,
    /// `GET /QuickConnect/Connect`, on Jellyfin.
    QuickConnected,
    /// `POST /Users/AuthenticateWithQuickConnect`, on Jellyfin.
    QuickConnectSignIn,
    /// `GET` or `HEAD /UserImage`, on Jellyfin.
    Avatar,
    /// `GET` or `HEAD /Users/{id}/Images/Primary`, on Emby.
    UserAvatar(String),
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
    /// `GET /Items/Latest`, on Jellyfin.
    Latest,
    /// `GET /Users/{id}/Items/Latest`, on Emby.
    UserLatest(String),
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

/// The call `asked` makes on the route to the media server `kind` names, Jellyfin or Emby,
/// where it is one on the list.
pub(crate) fn listed(kind: Kind, asked: &Asked) -> Option<Call> {
    let segments = asked.segments();
    let emby = kind == Kind::Emby;
    let call = match (asked.method.clone(), segments.as_slice()) {
        (Method::GET, ["system", "info", "public"]) => Call::PublicInfo,
        (Method::POST, ["users", "authenticatebyname"]) => Call::SignIn,
        (Method::POST, ["quickconnect", "initiate"]) if !emby => Call::QuickConnect,
        (Method::GET, ["quickconnect", "connect"]) if !emby => Call::QuickConnected,
        (Method::POST, ["users", "authenticatewithquickconnect"]) if !emby => {
            Call::QuickConnectSignIn
        }
        (Method::GET | Method::HEAD, ["userimage"]) if !emby => Call::Avatar,
        (Method::GET | Method::HEAD, ["users", id, "images", "primary"]) if emby => {
            Call::UserAvatar(identifier(id)?)
        }
        (Method::GET, ["system", "info"]) => Call::Info,
        (Method::GET, ["users"]) => Call::Users,
        (Method::GET, ["users", id]) => Call::User(identifier(id)?),
        (Method::GET, ["users", id, "views"]) => Call::Views(identifier(id)?),
        (Method::GET, ["library", "mediafolders"]) => Call::MediaFolders,
        (Method::GET, ["items"]) => Call::Items,
        (Method::GET, ["items", "latest"]) if !emby => Call::Latest,
        (Method::GET, ["users", id, "items", "latest"]) if emby => {
            Call::UserLatest(identifier(id)?)
        }
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

/// The parameters a listing of what was added lately carries on Emby, which names the
/// account in the path.
const USER_LATEST: &[(&str, Shape)] = &[("Limit", Shape::Text), ("ParentId", Shape::Text)];

/// The one parameter an avatar carries on Emby: the quality it is asked at.
const USER_AVATAR: &[(&str, Shape)] = &[("quality", Shape::Integer)];

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
                | Self::UserAvatar(_)
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
            Self::UserAvatar(id) => Plan::Forward(
                Built::new(asked.method.clone(), format!("/Users/{id}/Images/Primary"))
                    .query(parameters(query, USER_AVATAR)?),
            ),
            Self::QuickConnect => Plan::Forward(on_device(
                reach,
                asked,
                Built::new(Method::POST, "/QuickConnect/Initiate"),
            )?),
            Self::QuickConnected => {
                let built = Built::new(Method::GET, "/QuickConnect/Connect");
                Plan::Forward(
                    on_device(reach, asked, built)?
                        .query(parameters(query, &[("secret", Shape::Text)])?),
                )
            }
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
            Self::UserLatest(id) => Plan::Forward(
                keyed(reach, Method::GET, &format!("/Users/{id}/Items/Latest"))?
                    .query(parameters(query, USER_LATEST)?),
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

/// `method` on `path` of `reach`'s media server, under the gate's own key.
fn keyed(reach: &Reach<'_>, method: Method, path: &str) -> Result<Built, Stop> {
    identified(
        Built::new(method, path),
        reach.upstream.kind,
        GATE,
        GATE,
        Some(reach.upstream.credential.reveal()),
    )
}

/// `built`, sent to `reach`'s media server from the request service's device `asked` names,
/// which must be one it assigns.
fn on_device(reach: &Reach<'_>, asked: &Asked, built: Built) -> Result<Built, Stop> {
    let device = seerrs(asked.authorisation.get(DEVICE_ID))?;
    identified(built, reach.upstream.kind, SEERR, &device, None)
}

/// `built`, from the client `client` on the device `device` and carrying `token` where
/// there is one, as the media server `kind` names reads them: Jellyfin in its
/// `MediaBrowser` authorisation, Emby in its `Emby` authorisation and its token header.
fn identified(
    built: Built,
    kind: Kind,
    client: &str,
    device: &str,
    token: Option<&str>,
) -> Result<Built, Stop> {
    let named = format!(
        "Client=\"{client}\", Device=\"{client}\", {DEVICE_ID}=\"{device}\", Version=\"{VERSION}\""
    );
    if kind == Kind::Emby {
        let built = built.header(EMBY_AUTHORISATION, value(&format!("{EMBY_SCHEME}{named}"))?);
        return match token {
            Some(token) => Ok(built.header(EMBY_TOKEN, secret(token)?)),
            None => Ok(built),
        };
    }
    match token {
        Some(token) => Ok(built.header(
            header::AUTHORIZATION,
            secret(&format!("{SCHEME}{named}, {TOKEN}=\"{token}\""))?,
        )),
        None => Ok(built.header(header::AUTHORIZATION, value(&format!("{SCHEME}{named}"))?)),
    }
}

/// `text` as a header's value.
fn value(text: &str) -> Result<HeaderValue, Stop> {
    HeaderValue::from_str(text).map_err(|_| Stop::Unreachable)
}

/// `text`, which carries a token, as a header's value marked as one not to show.
fn secret(text: &str) -> Result<HeaderValue, Stop> {
    let mut value = value(text)?;
    value.set_sensitive(true);
    Ok(value)
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
    let device = seerrs(asked.authorisation.get(DEVICE_ID))?;
    let kind = reach.upstream.kind;
    let answer = reach
        .send(identified(built, kind, SEERR, &device, None)?)
        .await?;
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
    let logout = Built::new(Method::POST, "/Sessions/Logout");
    let logout = identified(logout, kind, SEERR, &device, Some(token))?;
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
