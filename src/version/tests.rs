use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use lemonfiber_sidecar::gate::{Credential, File, Upstreams};
use serde_json::json;
use tokio::net::TcpListener;

use crate::fake::{ask, json, status, upstream, Answered, Config, JELLYFIN_TOKEN, TOKEN};
use crate::serving::Service;

/// Ask `service` for the tags on the Sonarr route.
async fn tags(service: &Arc<Service>) -> Answered {
    ask(
        Arc::clone(service),
        Method::GET,
        &format!("/sonarr/api/v3/tag?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await
}

/// Ask `service` for the media server's users.
async fn users(service: &Arc<Service>) -> Answered {
    let authorisation = format!("MediaBrowser Client=\"Seerr\", Token=\"{JELLYFIN_TOKEN}\"");
    ask(
        Arc::clone(service),
        Method::GET,
        "/jellyfin/Users",
        &[("authorization", authorisation.as_str())],
        Body::empty(),
    )
    .await
}

/// Rewrite `config`'s route `route` with `changed`.
fn rewrite(
    config: &Config,
    route: &str,
    changed: impl FnOnce(&mut lemonfiber_sidecar::gate::Upstream),
) {
    let mut upstreams = Upstreams::read(&config.read(File::Upstreams))
        .unwrap_or_else(|_| Upstreams::of(Vec::new()));
    if let Some(one) = upstreams
        .upstreams
        .iter_mut()
        .find(|one| one.route == route)
    {
        changed(one);
    }
    config.write(File::Upstreams, &upstreams.written());
}

#[tokio::test]
async fn an_arr_is_asked_once_and_again_only_when_its_route_changes() {
    let fake = upstream(vec![json(Method::GET, "/api/v3/tag", &json!([]))]).await;
    let config = Config::new("arr-asked").with_routes(&fake.address);
    let service = config.service();

    let first = tags(&service).await;
    let second = tags(&service).await;
    rewrite(&config, "sonarr", |one| {
        one.credential = Credential::new("a-new-key");
    });
    let third = tags(&service).await;

    for answered in [first, second, third] {
        assert_eq!(answered.status, StatusCode::OK);
    }
    assert_eq!(fake.sent("GET", "/api/v3/system/status").len(), 2);
    assert_eq!(fake.sent("GET", "/api/v3/tag").len(), 3);
}

#[tokio::test]
async fn an_arr_that_cannot_say_its_status_hands_back_why_and_is_asked_again() {
    let fake = upstream(vec![
        status(
            Method::GET,
            "/api/v3/system/status",
            StatusCode::UNAUTHORIZED,
        ),
        json(Method::GET, "/api/v3/tag", &json!([])),
    ])
    .await;
    let config = Config::new("arr-unanswered").with_routes(&fake.address);
    let service = config.service();

    let first = tags(&service).await;
    let second = tags(&service).await;

    assert_eq!(
        (first.status, second.status),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
    );
    assert_eq!(fake.sent("GET", "/api/v3/system/status").len(), 2);
    assert!(fake.sent("GET", "/api/v3/tag").is_empty());
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_media_server_on_a_supported_major_is_asked_once() {
    let fake = upstream(vec![json(Method::GET, "/Users", &json!([]))]).await;
    let config = Config::new("jellyfin-supported").with_routes(&fake.address);
    let service = config.service();

    let first = users(&service).await;
    let second = users(&service).await;

    assert_eq!(
        (first.status, second.status),
        (StatusCode::OK, StatusCode::OK)
    );
    assert_eq!(fake.sent("GET", "/System/Info/Public").len(), 1);
    assert_eq!(fake.sent("GET", "/Users").len(), 2);
}

#[tokio::test]
async fn every_call_on_a_media_server_of_another_major_is_answered_503_naming_it() {
    let fake = upstream(vec![
        json(
            Method::GET,
            "/System/Info/Public",
            &json!({ "Version": "12.1.0" }),
        ),
        json(Method::GET, "/Users", &json!([])),
    ])
    .await;
    let config = Config::new("jellyfin-unsupported").with_routes(&fake.address);
    let service = config.service();

    let first = users(&service).await;
    let second = users(&service).await;

    for answered in [&first, &second] {
        assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            answered.body,
            "Jellyfin 12.1.0 is not a version this gate supports"
        );
        assert_eq!(
            answered
                .headers
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/plain; charset=utf-8")
        );
    }
    assert_eq!(fake.sent("GET", "/System/Info/Public").len(), 1);
    assert!(fake.sent("GET", "/Users").is_empty());
    assert!(config.recorded().is_empty());

    rewrite(&config, "jellyfin", |one| {
        one.majors = vec![10, 12];
    });
    assert_eq!(users(&service).await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_media_server_route_naming_no_major_supports_none() {
    let fake = upstream(vec![json(Method::GET, "/Users", &json!([]))]).await;
    let config = Config::new("jellyfin-no-majors").with_routes(&fake.address);
    rewrite(&config, "jellyfin", |one| {
        one.majors = Vec::new();
    });

    let answered = users(&config.service()).await;

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answered.body,
        "Jellyfin 10.11.11 is not a version this gate supports"
    );
    assert!(fake.sent("GET", "/Users").is_empty());
}

#[tokio::test]
async fn a_version_that_does_not_start_with_a_number_is_not_supported() {
    let fake = upstream(vec![json(
        Method::GET,
        "/System/Info/Public",
        &json!({ "Version": "unstable" }),
    )])
    .await;
    let config = Config::new("jellyfin-unnumbered").with_routes(&fake.address);

    let answered = users(&config.service()).await;

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answered.body,
        "Jellyfin unstable is not a version this gate supports"
    );
}

#[tokio::test]
async fn a_media_server_that_does_not_say_its_version_is_a_bad_gateway() {
    let fake = upstream(vec![json(
        Method::GET,
        "/System/Info/Public",
        &json!({ "ServerName": "home" }),
    )])
    .await;
    let config = Config::new("jellyfin-silent").with_routes(&fake.address);

    let silent = users(&config.service()).await;

    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let address = listener
        .local_addr()
        .map(|at| format!("http://{at}"))
        .unwrap_or_default();
    drop(listener);
    let gone = Config::new("jellyfin-gone").with_routes(&address);
    let unreachable = users(&gone.service()).await;

    assert_eq!(silent.status, StatusCode::BAD_GATEWAY);
    assert_eq!(unreachable.status, StatusCode::BAD_GATEWAY);
    assert!(fake.sent("GET", "/Users").is_empty());
}
