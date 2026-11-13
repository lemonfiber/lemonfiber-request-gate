//! plex.tv, as the gate meets it: the one place outside the stack it reaches, and the
//! calls it answers in plex.tv's place in `fronted` mode.
//!
//! The gate reaches plex.tv's `/users/account.json` alone. In `owner` mode it asks whose
//! a token the request service presents is, once per token, keeping the token only as its
//! hash beside the answer. In `fronted` mode it passes on a member's own sign-in there and
//! hands back what came of it, under the address `<plex.tv id>@plex.invalid`.
//!
//! In `fronted` mode the request service resolves plex.tv to the gate, which answers it
//! from what the core applied: the request service's own account, the household's accounts
//! with this server, and this server. Every account it answers carries the address
//! `<plex.tv id>@plex.invalid`, never one the household holds.

use std::collections::HashMap;
use std::fmt::Write as _;

use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lemonfiber_ports::service::{OutsideAccount, OutsideServer, PlexLink, PlexMember};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::asked::Asked;
use crate::formats::TokenHash;
use crate::plex::TOKEN;
use crate::upstream::{passed, Stop};

/// Where plex.tv answers.
pub(crate) const BASE: &str = "https://plex.tv";

/// The one path the gate reaches plex.tv on.
const ACCOUNT: &str = "/users/account.json";

/// The host the request service asks plex.tv's own calls of.
const PLEX_TV: &str = "plex.tv";

/// The domain every address the gate answers for an account is under, which no mail
/// reaches.
const NO_MAIL: &str = "plex.invalid";

/// Where plex.tv's account answer holds the account.
const USER: &str = "user";

/// Where the account carries its address.
const EMAIL: &str = "email";

/// Where plex.tv's account answer holds the account's id.
const USER_ID: &str = "/user/id";

/// Where plex.tv's account answer holds the account's address.
const USER_EMAIL: &str = "/user/email";

/// How many tokens' owners are kept before every one is asked again.
const KEPT: usize = 256;

/// What the gate calls plex.tv's front in its record.
pub(crate) const ROUTE: &str = "plex.tv";

/// plex.tv, reached at `base`, and whose each token it was asked about is.
pub(crate) struct PlexTv {
    base: String,
    owners: Mutex<HashMap<TokenHash, Option<String>>>,
}

impl PlexTv {
    /// plex.tv, reached at `base`.
    pub(crate) fn at(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            owners: Mutex::new(HashMap::new()),
        }
    }

    /// The answer to `/users/account.json` asked with `token`.
    async fn account(
        &self,
        client: &reqwest::Client,
        token: &str,
    ) -> Result<reqwest::Response, Stop> {
        client
            .get(format!("{}{ACCOUNT}", self.base))
            .header(TOKEN, token)
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|_| Stop::Unreachable)
    }

    /// The plex.tv id of the account `token` is, none where plex.tv refuses it, asked once
    /// per token.
    pub(crate) async fn owner(
        &self,
        client: &reqwest::Client,
        token: &str,
    ) -> Result<Option<String>, Stop> {
        let hash = TokenHash::of(token);
        if let Some(known) = self.owners.lock().await.get(&hash) {
            return Ok(known.clone());
        }
        let answer = self.account(client, token).await?;
        let owner = if answer.status() == StatusCode::UNAUTHORIZED {
            None
        } else if answer.status().is_success() {
            let read: Value = answer.json().await.map_err(|_| Stop::Unreachable)?;
            Some(
                read.pointer(USER_ID)
                    .and_then(id)
                    .ok_or(Stop::Unreachable)?,
            )
        } else {
            return Err(Stop::Upstream(answer.status()));
        };
        let mut owners = self.owners.lock().await;
        if owners.len() >= KEPT {
            owners.clear();
        }
        owners.insert(hash, owner.clone());
        Ok(owner)
    }

    /// A member's own sign-in, passed on to plex.tv unchanged, and what came of it: an
    /// account plex.tv answers carrying the address `<plex.tv id>@plex.invalid` in place of
    /// the member's own, and any other answer as plex.tv gave it.
    pub(crate) async fn passed_on(&self, client: &reqwest::Client, token: &str) -> Response {
        let Ok(answer) = self.account(client, token).await else {
            return StatusCode::BAD_GATEWAY.into_response();
        };
        if !answer.status().is_success() {
            return passed(answer);
        }
        let status = answer.status();
        let Ok(read) = answer.json::<Value>().await else {
            return StatusCode::BAD_GATEWAY.into_response();
        };
        match addressed(read) {
            Some(account) => (status, Json(account)).into_response(),
            None => StatusCode::BAD_GATEWAY.into_response(),
        }
    }
}

/// `account`, as plex.tv answers a member's own, with every place it carries the
/// member's address carrying `<plex.tv id>@plex.invalid` instead; none where it names no
/// account by an id.
fn addressed(mut account: Value) -> Option<Value> {
    let id = account.pointer(USER_ID).and_then(id)?;
    let held = account
        .pointer(USER_EMAIL)
        .and_then(Value::as_str)
        .map(str::to_owned);
    let address = address(&id);
    if let Some(held) = held.filter(|held| !held.is_empty()) {
        replaced(&mut account, &held, &address);
    }
    let user = account.get_mut(USER)?.as_object_mut()?;
    user.insert(EMAIL.to_owned(), Value::String(address));
    Some(account)
}

/// `value` with every string that is `held`, in any case, made `address`.
fn replaced(value: &mut Value, held: &str, address: &str) {
    match value {
        Value::String(text) if text.eq_ignore_ascii_case(held) => address.clone_into(text),
        Value::Array(items) => {
            for item in items {
                replaced(item, held, address);
            }
        }
        Value::Object(fields) => {
            for field in fields.values_mut() {
                replaced(field, held, address);
            }
        }
        _ => {}
    }
}

/// An account id, as plex.tv writes it in a number or in a string of digits.
fn id(value: &Value) -> Option<String> {
    let id = match value {
        Value::Number(number) => number.as_u64()?.to_string(),
        Value::String(text) => text.clone(),
        _ => return None,
    };
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then_some(id)
}

/// One call on plex.tv's front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Call {
    /// `GET /users/account.json`.
    Account,
    /// `GET /api/v2/ping`.
    Ping,
    /// `GET /api/users`.
    Users,
    /// `GET /api/resources`.
    Resources,
}

/// The call `asked` makes of plex.tv, where it is one on the list: asked of plex.tv
/// itself, by `GET`, at a path the list names.
pub(crate) fn listed(asked: &Asked, host: Option<&str>) -> Option<Call> {
    let host = host?.split(':').next()?;
    if !host.eq_ignore_ascii_case(PLEX_TV) || asked.method != Method::GET {
        return None;
    }
    let call = match asked.segments().as_slice() {
        ["users", "account.json"] => Call::Account,
        ["api", "v2", "ping"] => Call::Ping,
        ["api", "users"] => Call::Users,
        ["api", "resources"] => Call::Resources,
        _ => return None,
    };
    Some(call)
}

/// What plex.tv's front answers `call`, asked with the gate token `token`, from what the
/// core applied on the Plex route: none where the route is linked in another mode.
pub(crate) fn answered(call: Call, token: &str, link: &PlexLink) -> Option<Response> {
    let PlexLink::Fronted {
        account,
        members,
        server,
    } = link
    else {
        return None;
    };
    Some(match call {
        Call::Account => Json(json!({ USER: user(account, token)? })).into_response(),
        Call::Ping => Json(json!({ "pong": true })).into_response(),
        Call::Users => xml(&users(members, server)?),
        Call::Resources => xml(&resources(server)),
    })
}

/// `account` as plex.tv answers its own account, signed in with `token`.
fn user(account: &OutsideAccount, token: &str) -> Option<Value> {
    let id: u64 = account.id.parse().ok()?;
    Some(json!({
        "id": id,
        "uuid": account.uuid,
        "email": address(&account.id),
        "joined_at": "",
        "username": account.username,
        "title": account.title,
        "thumb": "",
        "hasPassword": true,
        "authToken": token,
        "subscription": { "active": false, "status": "Inactive", "plan": "free", "features": [] },
        "roles": { "roles": [] },
        "entitlements": [],
    }))
}

/// The address the gate answers for the account `id`, which no mail reaches.
fn address(id: &str) -> String {
    format!("{id}@{NO_MAIL}")
}

/// The household's accounts with this server, as plex.tv lists the accounts a server is
/// shared with, each by its name.
fn users(members: &[PlexMember], server: &OutsideServer) -> Option<String> {
    let mut listed = String::new();
    for member in members {
        member.id.parse::<u64>().ok()?;
        let _ = write!(
            listed,
            r#"<User id="{}" title="{}" username="{}" email="{}" thumb=""><Server id="{}" serverId="{}" machineIdentifier="{}" name="{}" lastSeenAt="0" numLibraries="0" allLibraries="1" owned="0" pending="0"/></User>"#,
            escaped(&member.id),
            escaped(&member.name),
            escaped(&member.name),
            escaped(&address(&member.id)),
            escaped(&member.id),
            escaped(&member.id),
            escaped(&server.id),
            escaped(&server.name),
        );
    }
    Some(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><MediaContainer friendlyName="myPlex" identifier="com.plexapp.plugins.myplex" machineIdentifier="{}" totalSize="{count}" size="{count}">{listed}</MediaContainer>"#,
        escaped(&server.id),
        count = members.len(),
    ))
}

/// This server alone, as plex.tv lists the resources an account reaches.
fn resources(server: &OutsideServer) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><MediaContainer size="1"><Device name="{}" product="Plex Media Server" productVersion="" platform="" platformVersion="" device="" clientIdentifier="{}" createdAt="0" lastSeenAt="0" provides="server" owned="1" presence="1"/></MediaContainer>"#,
        escaped(&server.name),
        escaped(&server.id),
    )
}

/// `text` as an XML attribute's value.
fn escaped(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, mark| {
        match mark {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
        out
    })
}

/// `body` as plex.tv's XML answer.
fn xml(body: &str) -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/xml; charset=utf-8"),
        )],
        body.to_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests;
