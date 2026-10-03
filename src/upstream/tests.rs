use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use lemonfiber_sidecar::gate::{Credential, Kind, Upstream};

use super::{kept, Built, Reach, Stop};
use crate::fake::{raw, status, upstream};

fn route(address: &str) -> Upstream {
    Upstream {
        route: "sonarr".to_owned(),
        kind: Kind::Sonarr,
        address: address.to_owned(),
        credential: Credential::new("key"),
        majors: Vec::new(),
    }
}

#[tokio::test]
async fn an_address_that_is_not_one_cannot_be_reached() {
    let client = reqwest::Client::new();
    let upstream = route("not an address");
    let reach = Reach {
        client: &client,
        upstream: &upstream,
    };

    assert_eq!(
        reach.send(Built::new(Method::GET, "/x")).await.err(),
        Some(Stop::Unreachable)
    );
}

#[tokio::test]
async fn a_read_for_a_check_is_the_upstreams_answer_when_it_succeeds_and_can_be_read() {
    let fake = upstream(vec![
        raw(Method::GET, "/text", Vec::new(), "not json"),
        status(Method::GET, "/failing", StatusCode::SERVICE_UNAVAILABLE),
        raw(
            Method::GET,
            "/list",
            vec![("content-type", "application/json")],
            "[1]",
        ),
    ])
    .await;
    let client = reqwest::Client::new();
    let upstream = route(&format!("{}/", fake.address));
    let reach = Reach {
        client: &client,
        upstream: &upstream,
    };

    assert_eq!(
        reach.read(Built::new(Method::GET, "/text")).await,
        Err(Stop::Unreachable)
    );
    assert_eq!(
        reach.read(Built::new(Method::GET, "/failing")).await,
        Err(Stop::Upstream(StatusCode::SERVICE_UNAVAILABLE))
    );
    assert_eq!(
        reach.read(Built::new(Method::GET, "/list")).await,
        Ok(serde_json::json!([1]))
    );
    assert!(fake.seen().iter().all(|seen| seen.query.is_none()));
}

#[test]
fn only_the_headers_that_describe_the_body_are_kept() {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(header::SET_COOKIE, HeaderValue::from_static("a=b"));
    headers.insert(header::ETAG, HeaderValue::from_static("\"e\""));

    let kept = kept(&headers);

    assert_eq!(kept.len(), 2);
    assert!(!kept.contains_key(header::SET_COOKIE));
}
