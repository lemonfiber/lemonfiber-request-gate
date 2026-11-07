use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::{asked, call, gate, held_film, held_series, holdings};
use crate::fake::{curated, Answer, MOVIES_ROUTE, TV_ROUTE};

/// What the request service sends to add a series it does not hold.
fn series_added() -> Value {
    json!({
        "tvdbId": 81189,
        "title": "Breaking Bad",
        "qualityProfileId": 4,
        "languageProfileId": 1,
        "seasons": [
            { "seasonNumber": 1, "monitored": true },
            { "seasonNumber": 2, "monitored": false },
            { "seasonNumber": 3, "monitored": true },
        ],
        "tags": [],
        "seasonFolder": true,
        "monitored": true,
        "monitorNewItems": "all",
        "rootFolderPath": "/data/media/tv",
        "seriesType": "anime",
        "addOptions": { "ignoreEpisodesWithFiles": true, "searchForMissingEpisodes": true },
    })
}

/// What the request service sends to add a film it does not hold.
fn film_added() -> Value {
    json!({
        "title": "The Matrix",
        "qualityProfileId": 4,
        "profileId": 4,
        "titleSlug": "603",
        "minimumAvailability": "released",
        "tmdbId": 603,
        "year": 1999,
        "rootFolderPath": "/data/media/tv/",
        "monitored": true,
        "tags": [],
        "addOptions": { "searchForMovie": false },
    })
}

/// A curator holding the profiles and folders, whose catalogue holds `catalogued`, and that
/// answers a request with `requested`.
fn curator(catalogued: &Value, requested: &Value) -> Vec<Answer> {
    let mut answers = holdings();
    answers.push(curated("catalogued", catalogued));
    answers.push(curated("request", requested));
    answers
}

#[tokio::test]
async fn a_series_is_requested_with_the_seasons_monitored_anime_and_later_seasons_and_a_search() {
    let unheld = json!({ "reference": 81189, "id": null, "title": "Breaking Bad", "year": 2008,
        "monitored": false, "files": 0, "seasons": [] });
    let (config, fake) = gate("series-added", curator(&unheld, &held_series())).await;

    let answered = call(
        &config,
        Method::POST,
        TV_ROUTE,
        "/series",
        "",
        Some(&series_added()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.json().pointer("/id"), Some(&json!(12)));
    assert_eq!(
        asked(&fake, "request"),
        [json!({
            "kind": "tv",
            "request": {
                "reference": 81189,
                "root_folder": "/data/media/tv/",
                "quality_profile": 4,
                "seasons": [1, 3],
                "anime": true,
                "later_seasons": true,
                "search": true,
            },
        })]
    );
    assert_eq!(
        asked(&fake, "catalogued"),
        [json!({ "kind": "tv", "reference": 81189 })]
    );
}

#[tokio::test]
async fn a_film_is_requested_at_the_folder_as_the_curator_spells_it() {
    let (config, fake) = gate("film-added", curator(&json!(null), &held_film())).await;

    let answered = call(
        &config,
        Method::POST,
        MOVIES_ROUTE,
        "/movie",
        "",
        Some(&film_added()),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.json().pointer("/tmdbId"), Some(&json!(603)));
    assert_eq!(
        asked(&fake, "request"),
        [json!({
            "kind": "movies",
            "request": {
                "reference": 603,
                "root_folder": "/data/media/tv/",
                "quality_profile": 4,
                "seasons": [],
                "anime": false,
                "later_seasons": false,
                "search": false,
            },
        })]
    );
}

#[tokio::test]
async fn a_series_numbered_as_usual_with_no_options_is_requested_so() {
    let mut sent = series_added();
    if let Some(sent) = sent.as_object_mut() {
        sent.insert("seriesType".to_owned(), json!("standard"));
        sent.insert("monitorNewItems".to_owned(), json!("none"));
        sent.remove("addOptions");
        sent.remove("seasons");
    }
    let (config, fake) = gate("series-plain", curator(&json!(null), &held_series())).await;

    let answered = call(&config, Method::POST, TV_ROUTE, "/series", "", Some(&sent)).await;

    assert_eq!(answered.status, StatusCode::OK);
    let request = asked(&fake, "request");
    let request = request.first().and_then(|one| one.get("request"));
    for (field, value) in [
        ("seasons", json!([])),
        ("anime", json!(false)),
        ("later_seasons", json!(false)),
        ("search", json!(false)),
    ] {
        assert_eq!(
            request.and_then(|one| one.get(field)),
            Some(&value),
            "{field}"
        );
    }
}

#[tokio::test]
async fn an_addition_the_curator_could_not_take_as_sent_is_refused_and_asks_no_request() {
    let held = held_series();
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "profile-unheld",
            json!({ "qualityProfileId": 5 }),
            "unheld profile",
        ),
        (
            "folder-unheld",
            json!({ "rootFolderPath": "/elsewhere" }),
            "unheld folder",
        ),
        (
            "no-profile",
            json!({ "qualityProfileId": null }),
            "no profile",
        ),
        ("no-folder", json!({ "rootFolderPath": null }), "no folder"),
        ("no-reference", json!({ "tvdbId": null }), "no reference"),
        (
            "negative-reference",
            json!({ "tvdbId": -1 }),
            "a negative reference",
        ),
        (
            "text-type",
            json!({ "seriesType": 1 }),
            "a series type not in text",
        ),
        (
            "seasons-not-a-list",
            json!({ "seasons": {} }),
            "seasons not a list",
        ),
        (
            "season-not-an-object",
            json!({ "seasons": [1] }),
            "a season not an object",
        ),
        (
            "season-unnumbered",
            json!({ "seasons": [{ "monitored": true }] }),
            "a season without its number",
        ),
        (
            "season-number-in-text",
            json!({ "seasons": [{ "seasonNumber": "1", "monitored": true }] }),
            "a season numbered in text",
        ),
        (
            "season-too-large",
            json!({ "seasons": [{ "seasonNumber": 5_000_000_000_u64, "monitored": true }] }),
            "a season past any number",
        ),
        (
            "options-not-an-object",
            json!({ "addOptions": true }),
            "options not an object",
        ),
        (
            "search-not-a-flag",
            json!({ "addOptions": { "searchForMissingEpisodes": "yes" } }),
            "a search not a flag",
        ),
    ];
    for (name, changed, why) in cases {
        let mut sent = series_added();
        if let (Some(sent), Some(changed)) = (sent.as_object_mut(), changed.as_object()) {
            for (field, value) in changed {
                if value.is_null() {
                    sent.remove(field);
                } else {
                    sent.insert(field.clone(), value.clone());
                }
            }
        }
        let (config, fake) = gate(name, curator(&json!(null), &held)).await;

        let answered = call(&config, Method::POST, TV_ROUTE, "/series", "", Some(&sent)).await;

        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{why}");
        assert!(asked(&fake, "request").is_empty(), "{why}");
    }
}

#[tokio::test]
async fn an_addition_of_a_title_the_curator_holds_or_a_body_that_is_no_object_is_refused() {
    let (config, fake) = gate("already-held", curator(&held_series(), &held_series())).await;

    let held = call(
        &config,
        Method::POST,
        TV_ROUTE,
        "/series",
        "",
        Some(&series_added()),
    )
    .await;
    let unshaped = call(
        &config,
        Method::POST,
        TV_ROUTE,
        "/series",
        "",
        Some(&json!([])),
    )
    .await;

    assert_eq!(held.status, StatusCode::FORBIDDEN);
    assert_eq!(unshaped.status, StatusCode::FORBIDDEN);
    assert!(asked(&fake, "request").is_empty());
}
