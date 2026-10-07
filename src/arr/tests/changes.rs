//! A removal and a command each carry only what the gate names for them.

use axum::http::{Method, StatusCode};
use lemonfiber_sidecar::gate::{File, Outcome, Record};
use serde_json::json;

use super::{call, gate, only};
use crate::fake::{json, status};

#[tokio::test]
async fn a_removal_carries_the_id_and_its_two_flags_and_nothing_else() {
    let (config, fake) = gate(
        "removals",
        vec![
            status(Method::DELETE, "/api/v3/movie/7", StatusCode::OK),
            status(Method::DELETE, "/api/v3/series/5", StatusCode::OK),
        ],
    )
    .await;

    let film = call(
        &config,
        Method::DELETE,
        "radarr",
        "/movie/007",
        "deleteFiles=true&addImportExclusion=false&moveFiles=true",
        Some(&json!({ "anything": 1 })),
    )
    .await;
    let series = call(&config, Method::DELETE, "sonarr", "/series/5", "", None).await;

    assert_eq!(
        (film.status, series.status),
        (StatusCode::OK, StatusCode::OK)
    );
    let sent = only(&fake, "DELETE", "/api/v3/movie/7");
    assert_eq!(
        sent.query.as_deref(),
        Some("deleteFiles=true&addImportExclusion=false")
    );
    assert_eq!(sent.body, None);
    assert_eq!(only(&fake, "DELETE", "/api/v3/series/5").query, None);
    let outcomes: Vec<_> = Record::read(&config.read(File::Record))
        .unwrap_or_default()
        .entries
        .into_iter()
        .map(|entry| (entry.path, entry.outcome))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("/radarr/api/v3/movie/007".to_owned(), Outcome::Removed),
            ("/sonarr/api/v3/series/5".to_owned(), Outcome::Removed)
        ]
    );
}

#[tokio::test]
async fn a_command_is_a_search_for_what_was_asked_for_or_a_refresh_and_nothing_else() {
    let (config, fake) = gate(
        "commands",
        vec![json(Method::POST, "/api/v3/command", &json!({ "id": 1 }))],
    )
    .await;
    let allowed = [
        (
            "radarr",
            json!({ "name": "MoviesSearch", "movieIds": [7], "sendUpdatesToClient": true }),
            json!({ "name": "MoviesSearch", "movieIds": [7] }),
        ),
        (
            "sonarr",
            json!({ "name": "MissingEpisodeSearch", "seriesId": 5, "path": "/" }),
            json!({ "name": "MissingEpisodeSearch", "seriesId": 5 }),
        ),
        (
            "sonarr",
            json!({ "name": "RefreshMonitoredDownloads" }),
            json!({ "name": "RefreshMonitoredDownloads" }),
        ),
        (
            "radarr",
            json!({ "name": "RefreshMonitoredDownloads", "movieIds": [1] }),
            json!({ "name": "RefreshMonitoredDownloads" }),
        ),
    ];
    for (route, sent, built) in allowed {
        let answered = call(&config, Method::POST, route, "/command", "", Some(&sent)).await;
        assert_eq!(answered.status, StatusCode::OK, "{sent}");
        assert_eq!(fake.seen().pop().and_then(|seen| seen.body), Some(built));
    }

    let refused = [
        ("sonarr", json!({ "name": "MoviesSearch", "movieIds": [7] })),
        (
            "radarr",
            json!({ "name": "MissingEpisodeSearch", "seriesId": 5 }),
        ),
        ("radarr", json!({ "name": "MoviesSearch" })),
        (
            "radarr",
            json!({ "name": "MoviesSearch", "movieIds": ["7"] }),
        ),
        (
            "sonarr",
            json!({ "name": "MissingEpisodeSearch", "seriesId": -5 }),
        ),
        ("sonarr", json!({ "name": "DeleteSeries", "seriesId": 5 })),
        ("sonarr", json!({ "name": "ResetApiKey" })),
        ("sonarr", json!({ "seriesId": 5 })),
        ("sonarr", json!([{ "name": "RefreshMonitoredDownloads" }])),
    ];
    for (route, sent) in &refused {
        let answered = call(&config, Method::POST, route, "/command", "", Some(sent)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{sent}");
    }
    let unread = call(&config, Method::POST, "sonarr", "/command", "", None).await;

    assert_eq!(unread.status, StatusCode::FORBIDDEN);
    assert_eq!(fake.seen().len(), 4);
    assert_eq!(config.recorded().len(), refused.len() + 1);
}
