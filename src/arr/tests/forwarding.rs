use axum::http::{Method, StatusCode};
use lemonfiber_contract::wire::Kind;
use serde_json::json;

use super::{asked, call, gate};
use crate::fake::{curate_refused, curated, MOVIES_ROUTE, TV_ROUTE};

#[tokio::test]
async fn a_call_off_the_list_or_on_the_other_routes_list_is_refused_and_recorded() {
    let (config, fake) = gate("off-the-list", Vec::new()).await;

    for (method, route, path) in [
        (Method::GET, TV_ROUTE, "/languageprofile"),
        (Method::PUT, TV_ROUTE, "/episode/monitor"),
        (Method::POST, TV_ROUTE, "/tag"),
        (Method::GET, TV_ROUTE, "/movie"),
        (Method::GET, MOVIES_ROUTE, "/series"),
        (Method::GET, MOVIES_ROUTE, "/episode"),
        (Method::GET, TV_ROUTE, "/series/abc"),
        (Method::GET, TV_ROUTE, "/series/99999999999999999999"),
        (Method::DELETE, MOVIES_ROUTE, "/movie/-1"),
        (Method::GET, TV_ROUTE, "/series/12/extra"),
        (Method::PATCH, TV_ROUTE, "/series"),
    ] {
        let answered = call(&config, method.clone(), route, path, "", None).await;
        assert_eq!(
            answered.status,
            StatusCode::FORBIDDEN,
            "{method} {route}{path}"
        );
    }
    let unversioned = crate::fake::ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v4/system/status?apikey={}", crate::fake::TV_TOKEN),
        &[],
        axum::body::Body::empty(),
    )
    .await;

    assert_eq!(unversioned.status, StatusCode::FORBIDDEN);
    assert_eq!(config.recorded().len(), 12);
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn a_path_is_matched_whatever_its_case() {
    let (config, _fake) = gate("cased", Vec::new()).await;

    let answered = call(&config, Method::GET, "TV", "/System/Status", "", None).await;

    assert_eq!(answered.status, StatusCode::OK);
}

#[tokio::test]
async fn a_curator_that_does_not_take_requests_is_answered_503_naming_what_it_files() {
    let (config, fake) = gate(
        "untaken",
        vec![
            curate_refused("titles", Kind::Unsupported),
            curate_refused("quality_profiles", Kind::Unsupported),
        ],
    )
    .await;

    let series = call(&config, Method::GET, TV_ROUTE, "/series", "", None).await;
    let films = call(
        &config,
        Method::GET,
        MOVIES_ROUTE,
        "/qualityprofile",
        "",
        None,
    )
    .await;

    assert_eq!(series.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(series.body, "The curator filing tv does not take requests");
    assert_eq!(
        series
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(
        films.body,
        "The curator filing movies does not take requests"
    );
    assert_eq!(asked(&fake, "titles").len(), 1);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_curator_refusing_otherwise_is_a_bad_gateway() {
    for kind in [
        Kind::Refused,
        Kind::Unavailable,
        Kind::Unauthorised,
        Kind::Disallowed,
        Kind::UnknownOperation,
    ] {
        let (config, _fake) = gate(
            "refused-otherwise",
            vec![
                curate_refused("titles", kind),
                curated("search", &json!(null)),
            ],
        )
        .await;

        let answered = call(&config, Method::GET, TV_ROUTE, "/series", "", None).await;

        assert_eq!(answered.status, StatusCode::BAD_GATEWAY, "{kind:?}");
    }
}
