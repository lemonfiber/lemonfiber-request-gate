use axum::body::Body;
use axum::http::Method;
use serde_json::Value;

use crate::fake::{ask, body, upstream, Answer, Answered, Config, Fake, RADARR_TOKEN, TOKEN};

mod additions;
mod changes;
mod forwarding;
mod monitoring;
mod reads;

/// A gate whose routes all reach an upstream answering `answers`.
async fn gate(name: &str, answers: Vec<Answer>) -> (Config, Fake) {
    let fake = upstream(answers).await;
    let config = Config::new(name).with_routes(&fake.address);
    (config, fake)
}

/// Ask `config`'s gate `method` on `path` of `route`, with that route's token, the
/// query `query` and `sent` as the body.
async fn call(
    config: &Config,
    method: Method,
    route: &str,
    path: &str,
    query: &str,
    sent: Option<&Value>,
) -> Answered {
    let token = if route == "radarr" {
        RADARR_TOKEN
    } else {
        TOKEN
    };
    let separator = if query.is_empty() { "" } else { "&" };
    ask(
        config.service(),
        method,
        &format!("/{route}/api/v3{path}?apikey={token}{separator}{query}"),
        &[],
        sent.map_or_else(Body::empty, body),
    )
    .await
}

/// The one request `fake` was sent with `method` on `path`.
fn only(fake: &Fake, method: &str, path: &str) -> crate::fake::Seen {
    let sent = fake.sent(method, path);
    assert_eq!(sent.len(), 1, "{method} {path}: {:?}", fake.seen());
    sent.into_iter()
        .next()
        .unwrap_or_else(|| crate::fake::Seen {
            method: String::new(),
            path: String::new(),
            query: None,
            headers: axum::http::HeaderMap::new(),
            body: None,
        })
}
