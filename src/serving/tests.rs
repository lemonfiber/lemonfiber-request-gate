use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use crate::formats::gate::{File, Outcome, Record};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{answers_ok, healthy, routes, serve};
use crate::fake::{ask, curate, curated, upstream, Config, MOVIES_TOKEN, TV_TOKEN};

#[tokio::test]
async fn health_answers_ok_with_nothing_written() {
    let config = Config::new("health");

    let answered = ask(config.service(), Method::GET, "/health", &[], Body::empty()).await;

    assert_eq!(answered.status, StatusCode::OK);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn every_call_is_refused_until_the_core_has_written_the_routes() {
    let config = Config::new("unwritten");

    for path in [
        format!("/tv/api/v3/system/status?apikey={TV_TOKEN}"),
        "/jellyfin/Users/Me".to_owned(),
        "/".to_owned(),
    ] {
        let answered = ask(config.service(), Method::GET, &path, &[], Body::empty()).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn routes_that_cannot_be_read_answer_unavailable() {
    let config = Config::new("unreadable");
    config.write(File::Upstreams, "not the routes");

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v3/system/status?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_route_the_core_did_not_write_is_refused_and_recorded() {
    let fake = upstream(Vec::new()).await;
    let config = Config::new("unknown-route").with_upstream(&fake);

    for path in ["/lidarr/api/v1/system/status", "/", "/health/again"] {
        let answered = ask(config.service(), Method::GET, path, &[], Body::empty()).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    let posted = ask(
        config.service(),
        Method::POST,
        "/health",
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(posted.status, StatusCode::FORBIDDEN);
    assert_eq!(
        config.recorded(),
        [
            "GET /lidarr/api/v1/system/status",
            "GET /",
            "GET /health/again",
            "POST /health"
        ]
    );
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_refusal_is_recorded_without_its_query_string_and_its_token() {
    let fake = upstream(Vec::new()).await;
    let config = Config::new("refused").with_upstream(&fake);

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/tv/api/v3/tag?apikey={TV_TOKEN}"),
        &[],
        crate::fake::body(&json!({ "label": "ana" })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(answered.body.is_empty());
    let record = config.read(File::Record);
    assert!(!record.contains(TV_TOKEN), "{record}");
    assert!(!record.contains("ana"), "{record}");
    let entries = Record::read(&record).unwrap_or_default().entries;
    assert_eq!(
        entries
            .iter()
            .map(|one| (one.route.as_str(), one.path.as_str(), one.outcome))
            .collect::<Vec<_>>(),
        [("tv", "/tv/api/v3/tag", Outcome::Refused)]
    );
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_listed_call_without_the_routes_token_is_answered_401_and_sent_nowhere() {
    let fake = upstream(vec![curated("quality_profiles", &json!([]))]).await;
    let config = Config::new("unauthorised").with_upstream(&fake);

    for uri in [
        "/tv/api/v3/qualityprofile".to_owned(),
        "/tv/api/v3/qualityprofile?apikey=wrong".to_owned(),
        format!("/tv/api/v3/qualityprofile?apikey={MOVIES_TOKEN}"),
        format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}&apikey={TV_TOKEN}"),
    ] {
        let answered = ask(config.service(), Method::GET, &uri, &[], Body::empty()).await;
        assert_eq!(answered.status, StatusCode::UNAUTHORIZED, "{uri}");
    }

    assert!(fake.seen().is_empty());
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_token_file_that_cannot_be_read_accepts_nothing() {
    let fake = upstream(vec![curated("quality_profiles", &json!([]))]).await;
    let config = Config::new("tokens-unreadable").with_upstream(&fake);
    config.write(File::Tokens, "not tokens");

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::UNAUTHORIZED);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn an_adapter_that_does_not_answer_is_a_bad_gateway() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let address = listener
        .local_addr()
        .map(|at| format!("http://{at}"))
        .unwrap_or_default();
    drop(listener);
    let config = Config::new("unreachable").with_routes(&address);

    let read = ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;
    let checked = ask(
        config.service(),
        Method::PUT,
        &format!("/tv/api/v3/series?apikey={TV_TOKEN}"),
        &[],
        crate::fake::body(&json!({ "id": 5 })),
    )
    .await;

    assert_eq!(read.status, StatusCode::BAD_GATEWAY);
    assert_eq!(checked.status, StatusCode::BAD_GATEWAY);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn an_answer_outside_the_contract_is_a_bad_gateway() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let address = listener
        .local_addr()
        .map(|at| format!("http://{at}"))
        .unwrap_or_default();
    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut asked = [0u8; 1024];
            let _ = stream.read(&mut asked).await;
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await;
        }
    });
    let config = Config::new("outside-the-contract").with_routes(&address);

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_redirect_from_an_adapter_is_not_followed() {
    let elsewhere = upstream(vec![curated("quality_profiles", &json!([]))]).await;
    let fake = upstream(vec![crate::fake::redirect(
        Method::POST,
        curate("quality_profiles"),
        format!("{}{}", elsewhere.address, curate("quality_profiles")),
    )])
    .await;
    let config = Config::new("redirect").with_upstream(&fake);

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/tv/api/v3/qualityprofile?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
    assert!(elsewhere.seen().is_empty());
}

#[tokio::test]
async fn a_body_larger_than_the_gate_reads_is_refused() {
    let fake = upstream(Vec::new()).await;
    let config = Config::new("too-large").with_upstream(&fake);
    let large = "x".repeat(super::BODY + 1);

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/tv/api/v3/command?apikey={TV_TOKEN}"),
        &[],
        Body::from(large),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_removal_is_recorded_and_then_asked() {
    let fake = upstream(vec![curated("remove", &json!(null))]).await;
    let config = Config::new("removal").with_upstream(&fake);

    let answered = ask(
        config.service(),
        Method::DELETE,
        &format!(
            "/movies/api/v3/movie/7?apikey={MOVIES_TOKEN}&deleteFiles=true&addImportExclusion=false"
        ),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let asked = fake.sent("POST", curate("remove"));
    assert_eq!(
        asked.iter().map(|one| one.body.clone()).collect::<Vec<_>>(),
        [Some(json!({ "kind": "movies", "id": 7 }))]
    );
    let entries = Record::read(&config.read(File::Record))
        .unwrap_or_default()
        .entries;
    assert_eq!(
        entries
            .iter()
            .map(|one| (
                one.route.as_str(),
                one.method.as_str(),
                one.path.as_str(),
                one.outcome
            ))
            .collect::<Vec<_>>(),
        [(
            "movies",
            "DELETE",
            "/movies/api/v3/movie/7",
            Outcome::Removed
        )]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_removal_that_cannot_be_recorded_is_not_asked() {
    use std::os::unix::fs::PermissionsExt;

    let fake = upstream(vec![curated("remove", &json!(null))]).await;
    let config = Config::new("unrecorded-removal").with_upstream(&fake);
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o555));

    let answered = ask(
        config.service(),
        Method::DELETE,
        &format!("/tv/api/v3/series/5?apikey={TV_TOKEN}&deleteFiles=true&addImportExclusion=false"),
        &[],
        Body::empty(),
    )
    .await;
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o755));

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(fake.seen().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_refusal_that_cannot_be_recorded_is_still_refused() {
    use std::os::unix::fs::PermissionsExt;

    let fake = upstream(Vec::new()).await;
    let config = Config::new("unrecorded-refusal").with_upstream(&fake);
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o555));

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/tv/api/v3/tag?apikey={TV_TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o755));

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn the_health_check_reads_a_serving_service_as_healthy() {
    let config = Config::new("serving");
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    let app = routes(config.service());
    tokio::spawn(async move { axum::serve(listener, app).await });

    assert!(answers_ok(at).await);
    assert_eq!(healthy(at.port()).await, ExitCode::SUCCESS);
}

#[tokio::test]
async fn the_health_check_reads_nothing_listening_as_unhealthy() {
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = listener.local_addr() else {
        return;
    };
    drop(listener);

    assert!(!answers_ok(at).await);
    assert_eq!(healthy(at.port()).await, ExitCode::FAILURE);
}

#[tokio::test]
async fn serving_stops_when_told_to() {
    let config = Config::new("stops");

    let stopped = serve(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        routes(config.service()),
        std::future::ready(()),
    )
    .await;

    assert_eq!(stopped, ExitCode::SUCCESS);
}

#[tokio::test]
async fn an_address_already_taken_is_a_failure_to_serve() {
    let config = Config::new("taken");
    let Ok(taken) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await else {
        return;
    };
    let Ok(at) = taken.local_addr() else {
        return;
    };

    let refused = serve(at, routes(config.service()), std::future::ready(())).await;

    assert_eq!(refused, ExitCode::FAILURE);
}

#[tokio::test]
async fn the_plex_route_is_answered_on_its_own_port_alone() {
    let fake = upstream(Vec::new()).await;
    let config = Config::new("plex-on-the-routes").with_plex(
        &fake,
        lemonfiber_ports::service::PlexLink::Managed,
        Some(crate::fake::PLEX_TOKEN),
        Vec::new(),
    );

    let answered = ask(
        config.service(),
        Method::GET,
        "/plex/library/sections",
        &[("X-Plex-Token", crate::fake::PLEX_TOKEN)],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(fake.all().is_empty());
    assert_eq!(config.recorded(), ["GET /plex/library/sections"]);
}

#[tokio::test]
async fn a_status_an_upstream_answered_the_gates_own_read_with_is_handed_back() {
    let config = Config::new("upstream-status");
    let asked = crate::asked::Asked::new(
        Method::GET,
        &axum::http::Uri::from_static("/jellyfin/Users"),
        &axum::http::HeaderMap::new(),
        None,
    );

    let answered = super::stopped(
        &config.service(),
        &asked,
        crate::upstream::Stop::Upstream(StatusCode::NOT_FOUND),
    )
    .await;

    assert_eq!(answered.status(), StatusCode::NOT_FOUND);
    assert!(config.recorded().is_empty());
}
