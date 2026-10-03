use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use axum::body::Body;
use axum::http::{Method, StatusCode};
use lemonfiber_sidecar::gate::{File, Outcome, Record};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::{answers_ok, healthy, routes, serve};
use crate::fake::{ask, json, status, upstream, Config, RADARR_TOKEN, TOKEN};

#[tokio::test]
async fn health_answers_ok_with_nothing_written() {
    let config = Config::new("health");

    let answered = ask(config.service(), Method::GET, "/health", &[], Body::empty()).await;

    assert_eq!(answered.status, StatusCode::OK);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn nothing_is_answered_until_the_core_has_written_the_routes() {
    let config = Config::new("unwritten");

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/system/status?apikey={TOKEN}"),
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
    let config = Config::new("unknown-route").with_routes(&fake.address);

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
    let config = Config::new("refused").with_routes(&fake.address);

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/sonarr/api/v3/tag?apikey={TOKEN}"),
        &[],
        crate::fake::body(&json!({ "label": "ana" })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(answered.body.is_empty());
    let record = config.read(File::Record);
    assert!(!record.contains(TOKEN), "{record}");
    assert!(!record.contains("ana"), "{record}");
    let entries = Record::read(&record).unwrap_or_default().entries;
    assert_eq!(
        entries
            .iter()
            .map(|one| (one.route.as_str(), one.path.as_str(), one.outcome))
            .collect::<Vec<_>>(),
        [("sonarr", "/sonarr/api/v3/tag", Outcome::Refused)]
    );
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_listed_call_without_the_routes_token_is_answered_401_and_sent_nowhere() {
    let fake = upstream(vec![json(Method::GET, "/api/v3/system/status", &json!({}))]).await;
    let config = Config::new("unauthorised").with_routes(&fake.address);

    for uri in [
        "/sonarr/api/v3/system/status".to_owned(),
        "/sonarr/api/v3/system/status?apikey=wrong".to_owned(),
        format!("/sonarr/api/v3/system/status?apikey={RADARR_TOKEN}"),
        format!("/sonarr/api/v3/system/status?apikey={TOKEN}&apikey={TOKEN}"),
    ] {
        let answered = ask(config.service(), Method::GET, &uri, &[], Body::empty()).await;
        assert_eq!(answered.status, StatusCode::UNAUTHORIZED, "{uri}");
    }

    assert!(fake.seen().is_empty());
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_token_file_that_cannot_be_read_accepts_nothing() {
    let fake = upstream(vec![json(Method::GET, "/api/v3/system/status", &json!({}))]).await;
    let config = Config::new("tokens-unreadable").with_routes(&fake.address);
    config.write(File::Tokens, "not tokens");

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/system/status?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::UNAUTHORIZED);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn an_upstream_that_does_not_answer_is_a_bad_gateway() {
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
        &format!("/sonarr/api/v3/system/status?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;
    let checked = ask(
        config.service(),
        Method::PUT,
        &format!("/sonarr/api/v3/series?apikey={TOKEN}"),
        &[],
        crate::fake::body(&json!({ "id": 5 })),
    )
    .await;

    assert_eq!(read.status, StatusCode::BAD_GATEWAY);
    assert_eq!(checked.status, StatusCode::BAD_GATEWAY);
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn an_upstream_that_goes_away_after_saying_its_version_is_a_bad_gateway() {
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
    let config = Config::new("gone-after-version").with_routes(&address);

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/tag?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_read_the_gate_makes_for_its_checks_hands_back_the_upstreams_status() {
    let fake = upstream(vec![status(
        Method::GET,
        "/api/v3/series/5",
        StatusCode::NOT_FOUND,
    )])
    .await;
    let config = Config::new("check-status").with_routes(&fake.address);

    let answered = ask(
        config.service(),
        Method::PUT,
        &format!("/sonarr/api/v3/series?apikey={TOKEN}"),
        &[],
        crate::fake::body(&json!({ "id": 5, "monitored": true })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::NOT_FOUND);
    assert!(fake.sent("PUT", "/api/v3/series").is_empty());
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_redirect_is_handed_back_rather_than_followed() {
    let elsewhere = upstream(vec![json(Method::GET, "/api/v3/system/status", &json!({}))]).await;
    let fake = upstream(vec![crate::fake::redirect(
        Method::GET,
        "/api/v3/system/status",
        format!("{}/api/v3/system/status", elsewhere.address),
    )])
    .await;
    let config = Config::new("redirect").with_routes(&fake.address);

    let answered = ask(
        config.service(),
        Method::GET,
        &format!("/sonarr/api/v3/system/status?apikey={TOKEN}"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FOUND);
    assert!(elsewhere.seen().is_empty());
}

#[tokio::test]
async fn a_body_larger_than_the_gate_reads_is_refused() {
    let fake = upstream(Vec::new()).await;
    let config = Config::new("too-large").with_routes(&fake.address);
    let large = "x".repeat(super::BODY + 1);

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/sonarr/api/v3/command?apikey={TOKEN}"),
        &[],
        Body::from(large),
    )
    .await;

    assert_eq!(answered.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn a_removal_is_recorded_and_then_passed_on() {
    let fake = upstream(vec![status(
        Method::DELETE,
        "/api/v3/movie/7",
        StatusCode::OK,
    )])
    .await;
    let config = Config::new("removal").with_routes(&fake.address);

    let answered = ask(
        config.service(),
        Method::DELETE,
        &format!("/radarr/api/v3/movie/7?apikey={RADARR_TOKEN}&deleteFiles=true"),
        &[],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(fake.sent("DELETE", "/api/v3/movie/7").len(), 1);
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
            "radarr",
            "DELETE",
            "/radarr/api/v3/movie/7",
            Outcome::Removed
        )]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_removal_that_cannot_be_recorded_is_not_passed_on() {
    use std::os::unix::fs::PermissionsExt;

    let fake = upstream(vec![status(
        Method::DELETE,
        "/api/v3/series/5",
        StatusCode::OK,
    )])
    .await;
    let config = Config::new("unrecorded-removal").with_routes(&fake.address);
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o555));

    let answered = ask(
        config.service(),
        Method::DELETE,
        &format!("/sonarr/api/v3/series/5?apikey={TOKEN}&deleteFiles=true"),
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
    let config = Config::new("unrecorded-refusal").with_routes(&fake.address);
    let _ = std::fs::set_permissions(config.path(), std::fs::Permissions::from_mode(0o555));

    let answered = ask(
        config.service(),
        Method::POST,
        &format!("/sonarr/api/v3/tag?apikey={TOKEN}"),
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
        config.service(),
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

    let refused = serve(at, config.service(), std::future::ready(())).await;

    assert_eq!(refused, ExitCode::FAILURE);
}
