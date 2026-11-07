use axum::body::Body;
use axum::http::Method;
use serde_json::{json, Value};

use crate::fake::{
    ask, body, curate, upstream, Answer, Answered, Config, Fake, MOVIES_ROUTE, MOVIES_TOKEN,
    TV_TOKEN,
};

mod additions;
mod changes;
mod forwarding;
mod monitoring;
mod reads;

/// A gate whose routes all reach an adapter answering `answers`.
async fn gate(name: &str, answers: Vec<Answer>) -> (Config, Fake) {
    let fake = upstream(answers).await;
    let config = Config::new(name).with_upstream(&fake);
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
    let token = if route == MOVIES_ROUTE {
        MOVIES_TOKEN
    } else {
        TV_TOKEN
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

/// What `fake` was asked of `operation`, each as the body it was asked with.
fn asked(fake: &Fake, operation: &str) -> Vec<Value> {
    fake.sent("POST", curate(operation))
        .into_iter()
        .map(|seen| seen.body.unwrap_or_default())
        .collect()
}

/// A series the curator holds as 12, with its first season monitored and its second not.
fn held_series() -> Value {
    json!({
        "reference": 81189,
        "id": 12,
        "title": "Breaking Bad",
        "year": 2008,
        "monitored": true,
        "files": 7,
        "seasons": [
            { "number": 1, "monitored": true, "files": 7, "episodes": 7 },
            { "number": 2, "monitored": false, "files": 0, "episodes": 13 },
        ],
    })
}

/// A film the curator holds as 7, unmonitored and without its file.
fn held_film() -> Value {
    json!({
        "reference": 603,
        "id": 7,
        "title": "The Matrix",
        "year": 1999,
        "monitored": false,
        "files": 0,
        "seasons": [],
    })
}

/// The quality profiles and root folders a curator holds.
fn holdings() -> Vec<Answer> {
    vec![
        crate::fake::curated(
            "quality_profiles",
            &json!([{ "id": 4, "name": "HD-1080p" }]),
        ),
        crate::fake::curated(
            "root_folders",
            &json!([{ "id": "1", "path": "/data/media/tv/" }]),
        ),
    ]
}
