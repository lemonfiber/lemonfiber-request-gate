use axum::http::{Method, StatusCode};
use serde_json::json;

use super::{asked, call, gate, held_film, held_series};
use crate::fake::{curated, KEY, MOVIES_ROUTE, TV_ROUTE};

#[tokio::test]
async fn the_status_names_the_api_each_route_presents_and_asks_nobody() {
    let (config, fake) = gate("status", Vec::new()).await;

    let series = call(&config, Method::GET, TV_ROUTE, "/system/status", "", None).await;
    let films = call(
        &config,
        Method::GET,
        MOVIES_ROUTE,
        "/system/status",
        "",
        None,
    )
    .await;

    assert_eq!(series.status, StatusCode::OK);
    assert_eq!(series.json(), json!({ "version": "4.0.0", "urlBase": "" }));
    assert_eq!(films.json(), json!({ "version": "5.0.0", "urlBase": "" }));
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn the_tags_and_the_episodes_are_empty_and_ask_nobody() {
    let (config, fake) = gate("answered-itself", Vec::new()).await;

    let tags = call(&config, Method::GET, MOVIES_ROUTE, "/tag", "", None).await;
    let episodes = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/episode",
        "seriesId=12",
        None,
    )
    .await;
    let unshaped = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/episode",
        "seriesId=x",
        None,
    )
    .await;

    assert_eq!((tags.status, tags.json()), (StatusCode::OK, json!([])));
    assert_eq!(
        (episodes.status, episodes.json()),
        (StatusCode::OK, json!([]))
    );
    assert_eq!(unshaped.status, StatusCode::FORBIDDEN);
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn the_profiles_and_folders_are_the_curators_asked_under_the_request_key() {
    let (config, fake) = gate(
        "holdings",
        vec![
            curated("quality_profiles", &json!([{ "id": 4, "name": "HD" }])),
            curated(
                "root_folders",
                &json!([
                    { "id": "3", "path": "/data/media/tv" },
                    { "id": "anime", "path": "/data/media/anime" },
                ]),
            ),
        ],
    )
    .await;

    let profiles = call(&config, Method::GET, TV_ROUTE, "/qualityProfile", "", None).await;
    let folders = call(&config, Method::GET, TV_ROUTE, "/rootfolder", "", None).await;

    assert_eq!(profiles.json(), json!([{ "id": 4, "name": "HD" }]));
    assert_eq!(
        folders.json(),
        json!([
            { "id": 3, "path": "/data/media/tv", "unmappedFolders": [] },
            { "id": 2, "path": "/data/media/anime", "unmappedFolders": [] },
        ])
    );
    let seen = fake.sent("POST", crate::fake::curate("quality_profiles"));
    assert_eq!(
        seen.first()
            .and_then(|one| one.header("authorization"))
            .map(str::to_owned),
        Some(format!("Bearer {KEY}"))
    );
    assert_eq!(asked(&fake, "quality_profiles"), [json!({})]);
}

#[tokio::test]
async fn every_title_is_the_curators_titles_and_one_reference_picks_its_own() {
    let other = json!({
        "reference": 1, "id": 3, "title": "Other", "year": null,
        "monitored": false, "files": 0, "seasons": [],
    });
    let (config, fake) = gate(
        "titles",
        vec![curated("titles", &json!([held_series(), other]))],
    )
    .await;

    let all = call(&config, Method::GET, TV_ROUTE, "/series", "", None).await;
    let one = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/series",
        "tvdbId=81189",
        None,
    )
    .await;
    let unshaped = call(&config, Method::GET, TV_ROUTE, "/series", "tvdbId=x", None).await;
    let past = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/series",
        "tvdbId=18446744073709551615",
        None,
    )
    .await;

    assert_eq!(all.json().as_array().map(Vec::len), Some(2));
    assert_eq!(
        one.json(),
        json!([{
            "id": 12,
            "title": "Breaking Bad",
            "titleSlug": "81189",
            "year": 2008,
            "monitored": true,
            "tags": [],
            "images": [],
            "tvdbId": 81189,
            "statistics": {
                "seasonCount": 2,
                "episodeFileCount": 7,
                "episodeCount": 20,
                "totalEpisodeCount": 20,
            },
            "seasons": [
                {
                    "seasonNumber": 1,
                    "monitored": true,
                    "statistics": { "episodeFileCount": 7, "episodeCount": 7, "totalEpisodeCount": 7 },
                },
                {
                    "seasonNumber": 2,
                    "monitored": false,
                    "statistics": { "episodeFileCount": 0, "episodeCount": 13, "totalEpisodeCount": 13 },
                },
            ],
        }])
    );
    assert_eq!(unshaped.status, StatusCode::FORBIDDEN);
    assert_eq!(past.status, StatusCode::FORBIDDEN);
    assert_eq!(
        asked(&fake, "titles"),
        [json!({ "kind": "tv" }), json!({ "kind": "tv" })]
    );
}

#[tokio::test]
async fn one_title_is_the_one_held_and_one_not_held_is_not_found() {
    let (config, fake) = gate("held", vec![curated("held_title", &held_film())]).await;
    let (unheld, _gone) = gate("unheld", vec![curated("held_title", &json!(null))]).await;

    let film = call(&config, Method::GET, MOVIES_ROUTE, "/movie/7", "", None).await;
    let missing = call(&unheld, Method::GET, MOVIES_ROUTE, "/movie/8", "", None).await;

    assert_eq!(
        film.json(),
        json!({
            "id": 7,
            "title": "The Matrix",
            "titleSlug": "603",
            "year": 1999,
            "monitored": false,
            "tags": [],
            "images": [],
            "tmdbId": 603,
            "hasFile": false,
        })
    );
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(
        asked(&fake, "held_title"),
        [json!({ "kind": "movies", "id": 7 })]
    );
}

#[tokio::test]
async fn a_lookup_by_reference_is_the_catalogues_title_and_by_name_its_entries() {
    let (config, fake) = gate(
        "lookups",
        vec![
            curated("catalogued", &held_series()),
            curated(
                "lookup",
                &json!([{ "title": "Breaking Bad", "year": 2008, "reference": 81189, "held_as": null }]),
            ),
        ],
    )
    .await;

    let by_reference = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/series/lookup",
        "term=TVDB:81189",
        None,
    )
    .await;
    let by_name = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/series/lookup",
        "term=Breaking%20Bad",
        None,
    )
    .await;

    assert_eq!(by_reference.json().pointer("/0/id"), Some(&json!(12)));
    assert_eq!(
        by_name.json(),
        json!([{
            "title": "Breaking Bad",
            "titleSlug": "81189",
            "year": 2008,
            "tvdbId": 81189,
            "seasons": [],
            "images": [],
        }])
    );
    assert_eq!(
        asked(&fake, "catalogued"),
        [json!({ "kind": "tv", "reference": 81189 })]
    );
    assert_eq!(
        asked(&fake, "lookup"),
        [json!({ "kind": "tv", "term": "Breaking Bad" })]
    );
}

#[tokio::test]
async fn a_title_the_catalogue_does_not_hold_looks_up_as_none_and_a_held_entry_keeps_its_id() {
    let (config, _fake) = gate(
        "looked-up-nothing",
        vec![
            curated("catalogued", &json!(null)),
            curated(
                "lookup",
                &json!([{ "title": "Held", "year": null, "reference": 5, "held_as": 9 }]),
            ),
        ],
    )
    .await;

    let none = call(
        &config,
        Method::GET,
        MOVIES_ROUTE,
        "/movie/lookup",
        "term=tmdb:603",
        None,
    )
    .await;
    let held = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/series/lookup",
        "term=Held",
        None,
    )
    .await;

    assert_eq!((none.status, none.json()), (StatusCode::OK, json!([])));
    assert_eq!(held.json().pointer("/0/id"), Some(&json!(9)));
    assert_eq!(held.json().pointer("/0/year"), Some(&json!(0)));
}

#[tokio::test]
async fn a_film_lookup_by_name_or_a_reference_not_in_digits_is_refused() {
    let (config, fake) = gate("lookups-refused", Vec::new()).await;

    for (route, path, query) in [
        (MOVIES_ROUTE, "/movie/lookup", "term=The%20Matrix"),
        (MOVIES_ROUTE, "/movie/lookup", "term=tmdb:"),
        (MOVIES_ROUTE, "/movie/lookup", "term=tmdb:+603"),
        (TV_ROUTE, "/series/lookup", "term=tvdb:12a"),
        (TV_ROUTE, "/series/lookup", "term="),
        (TV_ROUTE, "/series/lookup", ""),
    ] {
        let answered = call(&config, Method::GET, route, path, query, None).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{route} {query}");
    }
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn the_queue_is_what_the_curator_is_downloading() {
    let (config, fake) = gate(
        "queue",
        vec![curated(
            "downloading",
            &json!([
                {
                    "title_id": 12,
                    "release": "Breaking.Bad.S02E01",
                    "bytes": 1000,
                    "left": 250,
                    "expected": 4_102_444_800_i64,
                    "state": "downloading",
                    "download_id": "SABnzbd_nzo_1",
                    "episode": { "season": 2, "number": 1, "title": "Seven Thirty-Seven" },
                },
                {
                    "title_id": 3,
                    "release": "Other",
                    "bytes": 10,
                    "left": 10,
                    "expected": null,
                    "state": "paused",
                    "download_id": null,
                    "episode": null,
                },
            ]),
        )],
    )
    .await;

    let queue = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/queue",
        "includeEpisode=true",
        None,
    )
    .await;
    let unshaped = call(
        &config,
        Method::GET,
        TV_ROUTE,
        "/queue",
        "includeEpisode=yes",
        None,
    )
    .await;

    let answered = queue.json();
    assert_eq!(answered.pointer("/totalRecords"), Some(&json!(2)));
    assert_eq!(
        answered.pointer("/records/0/estimatedCompletionTime"),
        Some(&json!("2100-01-01T00:00:00Z"))
    );
    assert_eq!(answered.pointer("/records/0/seriesId"), Some(&json!(12)));
    assert_eq!(
        answered.pointer("/records/0/episode"),
        Some(&json!({ "seasonNumber": 2, "episodeNumber": 1, "title": "Seven Thirty-Seven" }))
    );
    assert_eq!(
        answered.pointer("/records/1"),
        Some(&json!({
            "id": 2,
            "seriesId": 3,
            "title": "Other",
            "size": 10,
            "sizeleft": 10,
            "status": "paused",
        }))
    );
    assert_eq!(unshaped.status, StatusCode::FORBIDDEN);
    assert_eq!(asked(&fake, "downloading"), [json!({ "kind": "tv" })]);
}
