use axum::http::{Method, StatusCode};
use serde_json::json;

use super::{asked, call, gate};
use crate::fake::{curated, MOVIES_ROUTE, TV_ROUTE};

/// A removal's parameters as the request service sends them.
const REMOVED: &str = "deleteFiles=true&addImportExclusion=false";

#[tokio::test]
async fn a_removal_with_its_files_and_no_exclusion_asks_the_curator_to_remove() {
    let (config, fake) = gate("removed", vec![curated("remove", &json!(null))]).await;

    let answered = call(
        &config,
        Method::DELETE,
        TV_ROUTE,
        "/series/12",
        REMOVED,
        None,
    )
    .await;

    assert_eq!(
        (answered.status, answered.json()),
        (StatusCode::OK, json!({}))
    );
    assert_eq!(asked(&fake, "remove"), [json!({ "kind": "tv", "id": 12 })]);
    assert_eq!(config.recorded(), ["DELETE /tv/api/v3/series/12"]);
}

#[tokio::test]
async fn a_removal_keeping_files_excluding_imports_or_naming_neither_is_refused() {
    let (config, fake) = gate("removal-refused", vec![curated("remove", &json!(null))]).await;

    for query in [
        "",
        "deleteFiles=true",
        "addImportExclusion=false",
        "deleteFiles=false&addImportExclusion=false",
        "deleteFiles=true&addImportExclusion=true",
        "deleteFiles=yes&addImportExclusion=false",
        "deleteFiles=true&deleteFiles=true&addImportExclusion=false",
    ] {
        let answered = call(
            &config,
            Method::DELETE,
            MOVIES_ROUTE,
            "/movie/7",
            query,
            None,
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{query}");
    }
    assert!(asked(&fake, "remove").is_empty());
    assert!(config
        .recorded()
        .iter()
        .all(|entry| entry == "DELETE /movies/api/v3/movie/7"));
}

#[tokio::test]
async fn a_refresh_of_download_tracking_is_accepted_and_asks_nobody() {
    let (config, fake) = gate("refresh", Vec::new()).await;

    let answered = call(
        &config,
        Method::POST,
        MOVIES_ROUTE,
        "/command",
        "",
        Some(&json!({ "name": "RefreshMonitoredDownloads", "other": 1 })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::CREATED);
    assert_eq!(
        answered.json(),
        json!({ "name": "RefreshMonitoredDownloads" })
    );
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn a_search_asks_the_curator_to_search_each_title_named() {
    let (config, fake) = gate("searched", vec![curated("search", &json!(null))]).await;

    let series = call(
        &config,
        Method::POST,
        TV_ROUTE,
        "/command",
        "",
        Some(&json!({ "name": "MissingEpisodeSearch", "seriesId": 12 })),
    )
    .await;
    let films = call(
        &config,
        Method::POST,
        MOVIES_ROUTE,
        "/command",
        "",
        Some(&json!({ "name": "MoviesSearch", "movieIds": [7, 8] })),
    )
    .await;

    assert_eq!(
        (series.status, films.status),
        (StatusCode::CREATED, StatusCode::CREATED)
    );
    assert_eq!(films.json(), json!({ "name": "MoviesSearch" }));
    assert_eq!(
        asked(&fake, "search"),
        [
            json!({ "kind": "tv", "id": 12 }),
            json!({ "kind": "movies", "id": 7 }),
            json!({ "kind": "movies", "id": 8 }),
        ]
    );
}

#[tokio::test]
async fn any_other_command_or_one_on_the_other_route_is_refused() {
    let (config, fake) = gate("commands-refused", vec![curated("search", &json!(null))]).await;

    for (route, sent) in [
        (TV_ROUTE, json!({ "name": "MoviesSearch", "movieIds": [7] })),
        (
            MOVIES_ROUTE,
            json!({ "name": "MissingEpisodeSearch", "seriesId": 12 }),
        ),
        (TV_ROUTE, json!({ "name": "RescanSeries", "seriesId": 12 })),
        (TV_ROUTE, json!({ "name": "MissingEpisodeSearch" })),
        (
            TV_ROUTE,
            json!({ "name": "MissingEpisodeSearch", "seriesId": -1 }),
        ),
        (
            MOVIES_ROUTE,
            json!({ "name": "MoviesSearch", "movieIds": 7 }),
        ),
        (TV_ROUTE, json!({ "name": 1 })),
        (TV_ROUTE, json!([])),
    ] {
        let answered = call(&config, Method::POST, route, "/command", "", Some(&sent)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{route} {sent}");
    }
    assert!(fake.all().is_empty());
}
