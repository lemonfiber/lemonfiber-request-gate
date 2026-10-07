//! A film or series request is built from its named fields, at what the \*arr
//! holds, and added once.

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::{call, gate, only};
use crate::fake::{json, status, Answer};

/// What an \*arr holding quality profiles 1 and 2, the root folder `/data/<library>/`
/// and tags 1 and 3 answers a gate's checks with, and `added` to the addition.
fn holding(
    library: &'static str,
    folder: &'static str,
    held: &Value,
    added: Answer,
) -> Vec<Answer> {
    vec![
        json(
            Method::GET,
            "/api/v3/qualityProfile",
            &json!([{ "id": 1 }, { "id": 2 }]),
        ),
        json(
            Method::GET,
            "/api/v3/rootfolder",
            &json!([{ "id": 1, "path": folder }, { "id": 2 }]),
        ),
        json(
            Method::GET,
            "/api/v3/tag",
            &json!([{ "id": 1 }, { "id": 3 }]),
        ),
        json(Method::GET, library, held),
        added,
    ]
}

/// A film request as the request service sends it, with fields the gate drops.
fn film_request() -> Value {
    json!({
        "title": "The Matrix",
        "tmdbId": 603,
        "year": 1999,
        "titleSlug": "603",
        "qualityProfileId": 1,
        "profileId": 1,
        "minimumAvailability": "released",
        "rootFolderPath": "/data/movies",
        "monitored": true,
        "tags": [3],
        "addOptions": { "searchForMovie": true, "monitor": "movieOnly" },
        "path": "/etc",
        "id": 5,
        "movieFile": { "path": "/etc/passwd" }
    })
}

#[tokio::test]
async fn a_film_request_is_built_from_its_named_fields_at_what_radarr_holds() {
    let (config, fake) = gate(
        "film",
        holding(
            "/api/v3/movie",
            "/data/movies/",
            &json!([]),
            json(Method::POST, "/api/v3/movie", &json!({ "id": 9 })),
        ),
    )
    .await;

    let answered = call(
        &config,
        Method::POST,
        "radarr",
        "/movie",
        "",
        Some(&film_request()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.json(), json!({ "id": 9 }));
    assert_eq!(
        only(&fake, "GET", "/api/v3/movie").query.as_deref(),
        Some("tmdbId=603")
    );
    let sent = only(&fake, "POST", "/api/v3/movie");
    assert_eq!(sent.query, None);
    assert_eq!(
        sent.body,
        Some(json!({
            "title": "The Matrix",
            "tmdbId": 603,
            "year": 1999,
            "titleSlug": "603",
            "qualityProfileId": 1,
            "profileId": 1,
            "minimumAvailability": "released",
            "rootFolderPath": "/data/movies/",
            "monitored": true,
            "tags": [3],
            "addOptions": { "searchForMovie": true }
        }))
    );
}

#[tokio::test]
async fn a_film_request_off_what_radarr_holds_is_refused() {
    let (config, fake) = gate(
        "film-refused",
        holding(
            "/api/v3/movie",
            "/data/movies",
            &json!([]),
            json(Method::POST, "/api/v3/movie", &json!({ "id": 9 })),
        ),
    )
    .await;
    let changed = |field: &str, value: Value| {
        let mut sent = film_request();
        if let Some(fields) = sent.as_object_mut() {
            if value.is_null() {
                fields.remove(field);
            } else {
                fields.insert(field.to_owned(), value);
            }
        }
        sent
    };
    let refused = [
        changed("qualityProfileId", json!(4)),
        changed("qualityProfileId", Value::Null),
        changed("profileId", json!(4)),
        changed("rootFolderPath", json!("/data")),
        changed("rootFolderPath", json!("/data/movies/other")),
        changed("rootFolderPath", Value::Null),
        changed("tags", json!([2])),
        changed("tags", json!("3")),
        changed("tmdbId", Value::Null),
        changed("title", json!(7)),
        changed("year", json!(-1)),
        changed("addOptions", json!(true)),
        changed("addOptions", json!({ "searchForMovie": "yes" })),
        json!("a film"),
    ];

    for sent in &refused {
        let answered = call(&config, Method::POST, "radarr", "/movie", "", Some(sent)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{sent}");
    }
    assert!(fake.sent("POST", "/api/v3/movie").is_empty());
    assert_eq!(config.recorded().len(), refused.len());
}

#[tokio::test]
async fn a_film_radarr_holds_already_is_not_added_again() {
    let (config, fake) = gate(
        "film-held",
        holding(
            "/api/v3/movie",
            "/data/movies",
            &json!([{ "id": 2, "tmdbId": 603 }]),
            json(Method::POST, "/api/v3/movie", &json!({ "id": 9 })),
        ),
    )
    .await;

    let answered = call(
        &config,
        Method::POST,
        "radarr",
        "/movie",
        "",
        Some(&film_request()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(fake.sent("POST", "/api/v3/movie").is_empty());
}

#[tokio::test]
async fn what_radarr_answers_a_check_with_decides_it() {
    let lists = [
        ("/api/v3/qualityProfile", json!({ "id": 1 })),
        ("/api/v3/rootfolder", json!({ "path": "/data/movies" })),
        ("/api/v3/movie", json!({})),
    ];
    for (path, unreadable) in lists {
        let mut answers: Vec<Answer> = holding(
            "/api/v3/movie",
            "/data/movies",
            &json!([]),
            json(Method::POST, "/api/v3/movie", &json!({ "id": 9 })),
        );
        answers.insert(0, json(Method::GET, path, &unreadable));
        let (config, fake) = gate("film-unreadable", answers).await;

        let answered = call(
            &config,
            Method::POST,
            "radarr",
            "/movie",
            "",
            Some(&film_request()),
        )
        .await;

        assert_eq!(answered.status, StatusCode::BAD_GATEWAY, "{path}");
        assert!(fake.sent("POST", "/api/v3/movie").is_empty());
        assert!(config.recorded().is_empty());
    }

    let (config, _) = gate(
        "film-failing",
        vec![status(
            Method::GET,
            "/api/v3/qualityProfile",
            StatusCode::INTERNAL_SERVER_ERROR,
        )],
    )
    .await;
    let answered = call(
        &config,
        Method::POST,
        "radarr",
        "/movie",
        "",
        Some(&film_request()),
    )
    .await;
    assert_eq!(answered.status, StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn a_series_request_is_built_from_its_named_fields_at_what_sonarr_holds() {
    let (config, fake) = gate(
        "series",
        holding(
            "/api/v3/series",
            "/data/tv",
            &json!([]),
            status(Method::POST, "/api/v3/series", StatusCode::CREATED),
        ),
    )
    .await;
    let sent = json!({
        "tvdbId": 305_288,
        "title": "The Expanse",
        "qualityProfileId": 2,
        "languageProfileId": 1,
        "seasons": [
            { "seasonNumber": 1, "monitored": true, "statistics": {} },
            { "seasonNumber": 2, "monitored": false }
        ],
        "tags": [],
        "seasonFolder": true,
        "monitored": true,
        "monitorNewItems": "all",
        "rootFolderPath": "/data/tv/",
        "seriesType": "standard",
        "addOptions": { "ignoreEpisodesWithFiles": true, "searchForMissingEpisodes": false },
        "path": "/config"
    });

    let answered = call(&config, Method::POST, "sonarr", "/series", "", Some(&sent)).await;

    assert_eq!(answered.status, StatusCode::CREATED);
    assert_eq!(
        only(&fake, "GET", "/api/v3/series").query.as_deref(),
        Some("tvdbId=305288")
    );
    assert_eq!(
        only(&fake, "POST", "/api/v3/series").body,
        Some(json!({
            "tvdbId": 305_288,
            "title": "The Expanse",
            "qualityProfileId": 2,
            "languageProfileId": 1,
            "seasons": [
                { "seasonNumber": 1, "monitored": true },
                { "seasonNumber": 2, "monitored": false }
            ],
            "tags": [],
            "seasonFolder": true,
            "monitored": true,
            "monitorNewItems": "all",
            "rootFolderPath": "/data/tv",
            "seriesType": "standard",
            "addOptions": { "ignoreEpisodesWithFiles": true, "searchForMissingEpisodes": false }
        }))
    );
}

#[tokio::test]
async fn a_series_request_with_seasons_in_another_shape_is_refused() {
    let (config, fake) = gate(
        "series-refused",
        holding(
            "/api/v3/series",
            "/data/tv",
            &json!([]),
            status(Method::POST, "/api/v3/series", StatusCode::CREATED),
        ),
    )
    .await;
    let base = json!({ "tvdbId": 1, "qualityProfileId": 1, "rootFolderPath": "/data/tv" });
    let with_seasons = |seasons: Value| {
        let mut sent = base.clone();
        if let Some(fields) = sent.as_object_mut() {
            fields.insert("seasons".to_owned(), seasons);
        }
        sent
    };

    for sent in [
        with_seasons(json!(1)),
        with_seasons(json!([1])),
        with_seasons(json!([{ "seasonNumber": "1" }])),
    ] {
        let answered = call(&config, Method::POST, "sonarr", "/series", "", Some(&sent)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{sent}");
    }
    assert!(fake.seen().is_empty());

    let answered = call(&config, Method::POST, "sonarr", "/series", "", Some(&base)).await;
    assert_eq!(answered.status, StatusCode::CREATED);
}
