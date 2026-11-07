use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use crate::formats::gate::{Credential, File, Upstreams};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::json;
use tokio::net::TcpListener;

use crate::fake::{ask, curated, json, upstream, Answered, Config, JELLYFIN_TOKEN, TV_TOKEN};
use crate::serving::Service;

/// Ask `service` for the quality profiles on the series route.
async fn profiles(service: &Arc<Service>) -> Answered {
    ask(
        Arc::clone(service),
        Method::GET,
        &format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await
}

/// Ask `service` for the media server's users.
async fn users(service: &Arc<Service>) -> Answered {
    users_on(service, "jellyfin").await
}

/// Ask `service` for the media server's users on `route`.
async fn users_on(service: &Arc<Service>, route: &str) -> Answered {
    let authorisation = format!("MediaBrowser Client=\"Seerr\", Token=\"{JELLYFIN_TOKEN}\"");
    ask(
        Arc::clone(service),
        Method::GET,
        &format!("/{route}/Users"),
        &[("authorization", authorisation.as_str())],
        Body::empty(),
    )
    .await
}

/// Rewrite `config`'s route `route` with `changed`.
fn rewrite(
    config: &Config,
    route: &str,
    changed: impl FnOnce(&mut crate::formats::gate::Upstream),
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
async fn a_curators_route_is_asked_no_version() {
    let fake = upstream(vec![curated("quality_profiles", &json!([]))]).await;
    let config = Config::new("curator-unversioned").with_upstream(&fake);
    let service = config.service();

    let first = profiles(&service).await;
    rewrite(&config, "tv", |one| {
        one.credential = Credential::new("a-new-key");
    });
    let second = profiles(&service).await;

    for answered in [first, second] {
        assert_eq!(answered.status, StatusCode::OK);
    }
    assert_eq!(
        fake.all()
            .iter()
            .map(|one| one.path.as_str())
            .collect::<Vec<_>>(),
        [crate::fake::curate("quality_profiles"); 2]
    );
}

#[tokio::test]
async fn emby_on_another_major_is_answered_503_naming_emby() {
    let fake = upstream(vec![
        json(
            Method::GET,
            "/System/Info/Public",
            &json!({ "Version": "4.8.10.0" }),
        ),
        json(Method::GET, "/Users", &json!([])),
    ])
    .await;
    let config = Config::new("emby-unsupported").with_emby(&fake);

    let answered = users_on(&config.service(), "emby").await;

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answered.body,
        "Emby 4.8.10.0 is not a version this gate supports"
    );
    assert!(fake.sent("GET", "/Users").is_empty());
}

#[tokio::test]
async fn a_media_server_on_a_supported_major_is_asked_once() {
    let fake = upstream(vec![json(Method::GET, "/Users", &json!([]))]).await;
    let config = Config::new("jellyfin-supported").with_upstream(&fake);
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
    let config = Config::new("jellyfin-unsupported").with_upstream(&fake);
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
    let config = Config::new("jellyfin-no-majors").with_upstream(&fake);
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
    let config = Config::new("jellyfin-unnumbered").with_upstream(&fake);

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
    let config = Config::new("jellyfin-silent").with_upstream(&fake);

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

#[tokio::test]
async fn a_curators_route_is_supported_without_being_asked() {
    let client = crate::serving::client().unwrap_or_default();
    for kind in [
        crate::formats::gate::Kind::Tv,
        crate::formats::gate::Kind::Movies,
    ] {
        let upstream = crate::formats::gate::Upstream {
            route: "curator".to_owned(),
            kind,
            address: "http://127.0.0.1:1".to_owned(),
            credential: Credential::new("key"),
            majors: Vec::new(),
            pins: crate::formats::Pins::default(),
            link: None,
        };
        let reach = crate::upstream::Reach {
            client: &client,
            upstream: &upstream,
        };

        assert!(
            super::Versions::default().supported(&reach).await.is_ok(),
            "{kind:?}"
        );
    }
}
