use axum::body::Body;
use axum::http::{Method, StatusCode};
use lemonfiber_sidecar::gate::{File, Outcome, Record};
use serde_json::{json, Value};

use crate::fake::{
    ask, body, json, status, upstream, Answer, Answered, Config, Fake, KEY, RADARR_TOKEN, TOKEN,
};

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

/// Every read, as `(route, path, query)` sent and `(path, query)` sent upstream.
const READS: [(&str, &str, &str, &str, Option<&str>); 14] = [
    (
        "sonarr",
        "/system/status",
        "extra=1",
        "/api/v3/system/status",
        None,
    ),
    (
        "sonarr",
        "/QualityProfile",
        "",
        "/api/v3/qualityProfile",
        None,
    ),
    ("radarr", "/rootfolder", "", "/api/v3/rootfolder", None),
    ("radarr", "/tag", "", "/api/v3/tag", None),
    (
        "radarr",
        "/queue",
        "includeEpisode=TRUE&page=2",
        "/api/v3/queue",
        Some("includeEpisode=true"),
    ),
    (
        "sonarr",
        "/languageprofile",
        "",
        "/api/v3/languageprofile",
        None,
    ),
    (
        "radarr",
        "/movie",
        "tmdbId=0603",
        "/api/v3/movie",
        Some("tmdbId=603"),
    ),
    ("radarr", "/movie", "", "/api/v3/movie", None),
    ("radarr", "/movie/7", "", "/api/v3/movie/7", None),
    (
        "radarr",
        "/Movie/Lookup",
        "term=tmdb:603",
        "/api/v3/movie/lookup",
        Some("term=tmdb%3A603"),
    ),
    (
        "sonarr",
        "/series",
        "tvdbId=12",
        "/api/v3/series",
        Some("tvdbId=12"),
    ),
    ("sonarr", "/series/5", "", "/api/v3/series/5", None),
    (
        "sonarr",
        "/series/lookup",
        "term=The%20Expanse",
        "/api/v3/series/lookup",
        Some("term=The+Expanse"),
    ),
    (
        "sonarr",
        "/episode",
        "seriesId=5",
        "/api/v3/episode",
        Some("seriesId=5"),
    ),
];

#[tokio::test]
async fn every_read_goes_upstream_under_the_routes_key_with_only_its_named_parameters() {
    let answers = READS
        .iter()
        .map(|(_, _, _, upstream_path, _)| json(Method::GET, upstream_path, &json!([{ "id": 1 }])))
        .collect();
    let (config, fake) = gate("reads", answers).await;

    for (route, path, query, upstream_path, upstream_query) in READS {
        let answered = call(&config, Method::GET, route, path, query, None).await;
        assert_eq!(answered.status, StatusCode::OK, "{route} {path}");
        assert_eq!(answered.json(), json!([{ "id": 1 }]), "{route} {path}");
        assert_eq!(
            answered
                .headers
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        let seen = fake.seen().pop();
        assert_eq!(
            seen.as_ref().map(|one| one.path.as_str()),
            Some(upstream_path)
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.query.as_deref()),
            upstream_query,
            "{route} {path}"
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.header("x-api-key")),
            Some(KEY)
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.header("authorization")),
            None
        );
    }
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_named_parameter_sent_in_another_shape_or_twice_is_refused() {
    let (config, fake) = gate("parameters", Vec::new()).await;
    let refused = [
        (Method::GET, "radarr", "/queue", "includeEpisode=yes"),
        (Method::GET, "radarr", "/movie", "tmdbId=abc"),
        (Method::GET, "radarr", "/movie/lookup", "term=603"),
        (Method::GET, "radarr", "/movie/lookup", "term=tmdb:"),
        (Method::GET, "radarr", "/movie/lookup", "term=tmdb:6a"),
        (Method::GET, "radarr", "/movie/lookup", "term=tv"),
        (Method::GET, "radarr", "/movie/lookup", ""),
        (Method::GET, "sonarr", "/episode", "seriesId=-1"),
        (Method::GET, "sonarr", "/series", "tvdbId=1&tvdbId=2"),
        (Method::DELETE, "sonarr", "/series/5", "deleteFiles=yes"),
    ];

    for (method, route, path, query) in refused.clone() {
        let answered = call(&config, method, route, path, query, None).await;
        assert_eq!(
            answered.status,
            StatusCode::FORBIDDEN,
            "{route} {path} {query}"
        );
    }

    assert!(fake.seen().is_empty());
    assert_eq!(config.recorded().len(), refused.len());
}

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

#[tokio::test]
async fn the_request_services_headers_stay_behind() {
    let (config, fake) = gate(
        "headers",
        vec![json(Method::GET, "/api/v3/system/status", &json!({}))],
    )
    .await;

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/system/status?apikey={TOKEN}"),
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
    let sent = only(&fake, "GET", "/api/v3/system/status");
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
