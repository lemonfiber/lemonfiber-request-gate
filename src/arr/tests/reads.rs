//! Every read on the list goes upstream under the route's key with only the
//! parameters it names, and a named parameter in another shape is refused.

use axum::http::{Method, StatusCode};
use serde_json::json;

use super::{call, gate};
use crate::fake::{json, KEY};

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
        let seen = fake.all().pop();
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
