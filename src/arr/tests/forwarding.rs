//! What crosses the gate: no call off the list, none of the request service's
//! headers, and the upstream's answer as it answered.

use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::json;

use super::{call, gate, only};
use crate::fake::{ask, json, KEY, TOKEN};

#[tokio::test]
async fn a_call_off_the_list_or_on_the_other_arrs_route_is_refused() {
    let (config, fake) = gate("unlisted", Vec::new()).await;
    let refused = [
        (Method::POST, "sonarr", "/tag"),
        (Method::PUT, "radarr", "/tag/1"),
        (Method::GET, "sonarr", "/indexer"),
        (Method::POST, "radarr", "/downloadclient"),
        (Method::DELETE, "radarr", "/movie/abc"),
        (Method::GET, "radarr", "/movie/1/extra"),
        (Method::GET, "sonarr", "/movie"),
        (Method::POST, "sonarr", "/movie"),
        (Method::DELETE, "sonarr", "/movie/1"),
        (Method::GET, "radarr", "/series"),
        (Method::GET, "radarr", "/episode"),
        (Method::GET, "radarr", "/languageprofile"),
        (Method::PUT, "radarr", "/episode/monitor"),
        (Method::DELETE, "radarr", "/series/1"),
        (Method::GET, "sonarr", "/series/x"),
        (Method::PUT, "sonarr", "/system/status"),
    ];

    for (method, route, path) in refused.clone() {
        let answered = call(&config, method.clone(), route, path, "", None).await;
        assert_eq!(
            answered.status,
            StatusCode::FORBIDDEN,
            "{method} {route} {path}"
        );
    }
    let off_the_api = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v2/system/status?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(off_the_api.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
    assert_eq!(config.recorded().len(), refused.len() + 1);
}

#[tokio::test]
async fn the_request_services_headers_stay_behind() {
    let (config, fake) = gate(
        "headers",
        vec![json(Method::GET, "/api/v3/tag", &json!({}))],
    )
    .await;

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/tag?apikey={TOKEN}"),
        &[
            ("x-api-key", TOKEN),
            ("authorization", "Basic YWRtaW46YWRtaW4="),
            ("cookie", "session=1"),
            ("x-forwarded-for", "192.168.1.20"),
        ],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let sent = only(&fake, "GET", "/api/v3/tag");
    assert_eq!(sent.header("x-api-key"), Some(KEY));
    for header in ["authorization", "cookie", "x-forwarded-for"] {
        assert_eq!(sent.header(header), None, "{header}");
    }
}

#[tokio::test]
async fn what_the_upstream_answers_a_call_goes_back_as_it_answered() {
    let (config, _) = gate("not-found", Vec::new()).await;

    let answered = call(&config, Method::GET, "radarr", "/movie/9", "", None).await;

    assert_eq!(answered.status, StatusCode::NOT_FOUND);
    assert!(config.recorded().is_empty());
}
