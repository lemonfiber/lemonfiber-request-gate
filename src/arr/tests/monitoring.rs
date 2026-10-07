//! A film, series or episode the \*arr holds is monitored as it holds it, and
//! never unmonitored.

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::{call, gate, only};
use crate::fake::json;

#[tokio::test]
async fn episodes_are_only_ever_monitored() {
    let (config, fake) = gate(
        "episodes",
        vec![json(Method::PUT, "/api/v3/episode/monitor", &json!([]))],
    )
    .await;

    let answered = call(
        &config,
        Method::PUT,
        "sonarr",
        "/episode/monitor",
        "",
        Some(&json!({ "episodeIds": [1, 2], "monitored": true, "seriesId": 9 })),
    )
    .await;
    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        only(&fake, "PUT", "/api/v3/episode/monitor").body,
        Some(json!({ "episodeIds": [1, 2], "monitored": true }))
    );

    for sent in [
        json!({ "episodeIds": [1], "monitored": false }),
        json!({ "episodeIds": [1] }),
        json!({ "monitored": true }),
        json!({ "episodeIds": 1, "monitored": true }),
    ] {
        let answered = call(
            &config,
            Method::PUT,
            "sonarr",
            "/episode/monitor",
            "",
            Some(&sent),
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{sent}");
    }
    assert_eq!(fake.seen().len(), 1);
}

/// A film Radarr holds, unmonitored and without a file.
fn held_film() -> Value {
    json!({
        "id": 7,
        "title": "The Matrix",
        "path": "/data/movies/The Matrix (1999)",
        "rootFolderPath": "/data/movies",
        "monitored": false,
        "hasFile": false,
        "qualityProfileId": 1,
        "minimumAvailability": "released",
        "tags": [1]
    })
}

#[tokio::test]
async fn a_film_radarr_holds_is_monitored_as_radarr_holds_it() {
    let (config, fake) = gate(
        "film-monitored",
        vec![
            json(Method::GET, "/api/v3/movie/7", &held_film()),
            json(
                Method::GET,
                "/api/v3/qualityProfile",
                &json!([{ "id": 1 }, { "id": 2 }]),
            ),
            json(
                Method::GET,
                "/api/v3/tag",
                &json!([{ "id": 1 }, { "id": 3 }]),
            ),
            json(
                Method::PUT,
                "/api/v3/movie",
                &json!({ "id": 7, "monitored": true }),
            ),
        ],
    )
    .await;
    let sent = json!({
        "id": 7,
        "title": "Something else",
        "path": "/elsewhere",
        "rootFolderPath": "/elsewhere",
        "monitored": true,
        "hasFile": true,
        "qualityProfileId": 2,
        "minimumAvailability": "announced",
        "tags": [3, 1],
        "addOptions": { "searchForMovie": true, "monitor": "none" }
    });

    let answered = call(
        &config,
        Method::PUT,
        "radarr",
        "/movie",
        "moveFiles=true",
        Some(&sent),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let put = only(&fake, "PUT", "/api/v3/movie");
    assert_eq!(put.query, None);
    assert_eq!(
        put.body,
        Some(json!({
            "id": 7,
            "title": "The Matrix",
            "path": "/data/movies/The Matrix (1999)",
            "rootFolderPath": "/data/movies",
            "monitored": true,
            "hasFile": false,
            "qualityProfileId": 2,
            "minimumAvailability": "announced",
            "tags": [1, 3],
            "addOptions": { "searchForMovie": true }
        }))
    );
}

#[tokio::test]
async fn a_film_monitored_with_nothing_else_asked_changes_only_its_monitoring() {
    let mut held = held_film();
    if let Some(fields) = held.as_object_mut() {
        fields.remove("tags");
    }
    let (config, fake) = gate(
        "film-monitored-only",
        vec![
            json(Method::GET, "/api/v3/movie/7", &held),
            json(Method::GET, "/api/v3/tag", &json!([{ "id": 3 }])),
            json(Method::PUT, "/api/v3/movie", &json!({})),
        ],
    )
    .await;

    let monitored = call(
        &config,
        Method::PUT,
        "radarr",
        "/movie",
        "",
        Some(&json!({ "id": 7, "monitored": true })),
    )
    .await;
    let tagged = call(
        &config,
        Method::PUT,
        "radarr",
        "/movie",
        "",
        Some(&json!({ "id": 7, "tags": [3] })),
    )
    .await;

    assert_eq!(
        (monitored.status, tagged.status),
        (StatusCode::OK, StatusCode::OK)
    );
    let bodies: Vec<_> = fake
        .sent("PUT", "/api/v3/movie")
        .into_iter()
        .filter_map(|seen| seen.body)
        .collect();
    let mut expected = held.clone();
    if let Some(fields) = expected.as_object_mut() {
        fields.insert("monitored".to_owned(), json!(true));
    }
    let mut tagged_body = held;
    if let Some(fields) = tagged_body.as_object_mut() {
        fields.insert("tags".to_owned(), json!([3]));
    }
    assert_eq!(bodies, [expected, tagged_body]);
    assert!(fake.sent("GET", "/api/v3/qualityProfile").is_empty());
}

#[tokio::test]
async fn a_film_radarr_monitors_or_has_a_file_for_is_not_changed() {
    let held = |field: &str| {
        let mut film = held_film();
        if let Some(fields) = film.as_object_mut() {
            fields.insert(field.to_owned(), json!(true));
        }
        film
    };
    for (film, sent) in [
        (held("monitored"), json!({ "id": 7, "monitored": true })),
        (held("hasFile"), json!({ "id": 7, "monitored": true })),
        (held_film(), json!({ "id": 7, "qualityProfileId": 9 })),
        (held_film(), json!({ "id": 7, "tags": [2] })),
        (held_film(), json!({ "id": 7, "monitored": "yes" })),
        (held_film(), json!({ "id": 7, "addOptions": [] })),
        (held_film(), json!({ "monitored": true })),
    ] {
        let (config, fake) = gate(
            "film-unchanged",
            vec![
                json(Method::GET, "/api/v3/movie/7", &film),
                json(Method::GET, "/api/v3/qualityProfile", &json!([{ "id": 1 }])),
                json(Method::GET, "/api/v3/tag", &json!([{ "id": 1 }])),
            ],
        )
        .await;
        let answered = call(&config, Method::PUT, "radarr", "/movie", "", Some(&sent)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{film} {sent}");
        assert!(fake.sent("PUT", "/api/v3/movie").is_empty());
    }

    let (config, _) = gate(
        "film-unreadable",
        vec![json(Method::GET, "/api/v3/movie/7", &json!([held_film()]))],
    )
    .await;
    let answered = call(
        &config,
        Method::PUT,
        "radarr",
        "/movie",
        "",
        Some(&json!({ "id": 7 })),
    )
    .await;
    assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_series_sonarr_holds_gains_monitored_seasons_and_loses_nothing() {
    let held = json!({
        "id": 5,
        "title": "The Expanse",
        "path": "/data/tv/The Expanse",
        "monitored": false,
        "seasons": [
            { "seasonNumber": 1, "monitored": false },
            { "seasonNumber": 2, "monitored": false },
            { "seasonNumber": 3, "monitored": true },
            "unreadable"
        ],
        "tags": [1]
    });
    let (config, fake) = gate(
        "series-monitored",
        vec![
            json(Method::GET, "/api/v3/series/5", &held),
            json(
                Method::GET,
                "/api/v3/tag",
                &json!([{ "id": 1 }, { "id": 4 }]),
            ),
            json(Method::PUT, "/api/v3/series", &json!({ "id": 5 })),
        ],
    )
    .await;
    let sent = json!({
        "id": 5,
        "path": "/elsewhere",
        "monitored": true,
        "seasons": [
            { "seasonNumber": 2, "monitored": true },
            { "seasonNumber": 3, "monitored": false },
            { "seasonNumber": 9, "monitored": true }
        ],
        "tags": [4]
    });

    let answered = call(&config, Method::PUT, "sonarr", "/series", "", Some(&sent)).await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(
        only(&fake, "PUT", "/api/v3/series").body,
        Some(json!({
            "id": 5,
            "title": "The Expanse",
            "path": "/data/tv/The Expanse",
            "monitored": true,
            "seasons": [
                { "seasonNumber": 1, "monitored": false },
                { "seasonNumber": 2, "monitored": true },
                { "seasonNumber": 3, "monitored": true },
                "unreadable"
            ],
            "tags": [1, 4]
        }))
    );
}

#[tokio::test]
async fn a_series_is_never_unmonitored_and_seasons_it_lacks_change_nothing() {
    let held = json!({ "id": 5, "monitored": true, "path": "/data/tv/x" });
    let (config, fake) = gate(
        "series-unmonitored",
        vec![
            json(Method::GET, "/api/v3/series/5", &held),
            json(Method::PUT, "/api/v3/series", &json!({ "id": 5 })),
        ],
    )
    .await;

    let answered = call(
        &config,
        Method::PUT,
        "sonarr",
        "/series",
        "",
        Some(&json!({ "id": 5, "monitored": false, "seasons": [{ "seasonNumber": 1, "monitored": true }] })),
    )
    .await;
    let refused = call(
        &config,
        Method::PUT,
        "sonarr",
        "/series",
        "",
        Some(&json!({ "id": 5, "seasons": "all" })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(only(&fake, "PUT", "/api/v3/series").body, Some(held));
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}
