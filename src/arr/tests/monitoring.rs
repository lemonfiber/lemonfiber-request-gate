use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::{asked, call, gate, held_film, held_series};
use crate::fake::{curated, MOVIES_ROUTE, TV_ROUTE};

/// What the request service sends for more seasons of the series held as 12: the series as
/// the gate answered it, the seasons asked for monitored.
fn more_seasons() -> Value {
    json!({
        "id": 12,
        "tvdbId": 81189,
        "title": "Breaking Bad",
        "monitored": true,
        "tags": [],
        "seasons": [
            { "seasonNumber": 1, "monitored": true },
            { "seasonNumber": 2, "monitored": true },
        ],
    })
}

/// What the request service sends for the film held as 7.
fn film_monitored() -> Value {
    json!({
        "id": 7,
        "title": "The Matrix",
        "qualityProfileId": 4,
        "profileId": 4,
        "titleSlug": "603",
        "minimumAvailability": "released",
        "tmdbId": 603,
        "year": 1999,
        "tags": [],
        "rootFolderPath": "/data/media/movies",
        "monitored": true,
        "addOptions": { "searchForMovie": true },
    })
}

#[tokio::test]
async fn more_seasons_of_a_held_series_are_requested_by_its_reference() {
    let (config, fake) = gate(
        "more-seasons",
        vec![
            curated("held_title", &held_series()),
            curated("request", &held_series()),
        ],
    )
    .await;

    let answered = call(
        &config,
        Method::PUT,
        TV_ROUTE,
        "/series",
        "",
        Some(&more_seasons()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.json().pointer("/id"), Some(&json!(12)));
    assert_eq!(
        asked(&fake, "held_title"),
        [json!({ "kind": "tv", "id": 12 })]
    );
    assert_eq!(
        asked(&fake, "request"),
        [json!({
            "kind": "tv",
            "request": {
                "reference": 81189,
                "root_folder": "",
                "quality_profile": 0,
                "seasons": [1, 2],
                "anime": false,
                "later_seasons": false,
                "search": false,
            },
        })]
    );
}

#[tokio::test]
async fn a_held_film_unmonitored_without_its_file_is_requested_with_the_search_asked() {
    let mut requested = held_film();
    if let Some(requested) = requested.as_object_mut() {
        requested.insert("monitored".to_owned(), json!(true));
    }
    let (config, fake) = gate(
        "film-monitored",
        vec![
            curated("held_title", &held_film()),
            curated("request", &requested),
        ],
    )
    .await;

    let answered = call(
        &config,
        Method::PUT,
        MOVIES_ROUTE,
        "/movie",
        "",
        Some(&film_monitored()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.json().pointer("/monitored"), Some(&json!(true)));
    assert_eq!(
        asked(&fake, "request"),
        [json!({
            "kind": "movies",
            "request": {
                "reference": 603,
                "root_folder": "",
                "quality_profile": 0,
                "seasons": [],
                "anime": false,
                "later_seasons": false,
                "search": true,
            },
        })]
    );
}

#[tokio::test]
async fn a_held_film_monitored_or_with_its_file_is_refused() {
    for (name, changed) in [
        ("film-watched", json!({ "monitored": true })),
        ("film-filed", json!({ "files": 1 })),
    ] {
        let mut held = held_film();
        if let (Some(held), Some(changed)) = (held.as_object_mut(), changed.as_object()) {
            held.extend(changed.clone());
        }
        let (config, fake) = gate(
            name,
            vec![curated("held_title", &held), curated("request", &held)],
        )
        .await;

        let answered = call(
            &config,
            Method::PUT,
            MOVIES_ROUTE,
            "/movie",
            "",
            Some(&film_monitored()),
        )
        .await;

        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{name}");
        assert!(asked(&fake, "request").is_empty(), "{name}");
    }
}

#[tokio::test]
async fn a_title_the_curator_does_not_hold_is_not_found_and_requested_nowhere() {
    let (config, fake) = gate("monitor-unheld", vec![curated("held_title", &json!(null))]).await;

    let answered = call(
        &config,
        Method::PUT,
        TV_ROUTE,
        "/series",
        "",
        Some(&more_seasons()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::NOT_FOUND);
    assert!(asked(&fake, "request").is_empty());
}

#[tokio::test]
async fn a_change_naming_no_title_is_refused_and_asks_nobody() {
    let (config, fake) = gate("monitor-unnamed", Vec::new()).await;

    for sent in [
        json!({ "seasons": [] }),
        json!({ "id": "12" }),
        json!({ "id": 12, "addOptions": [] }),
    ] {
        let answered = call(
            &config,
            Method::PUT,
            MOVIES_ROUTE,
            "/movie",
            "",
            Some(&sent),
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{sent}");
    }
    assert!(fake.all().is_empty());
}
