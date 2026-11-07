use axum::http::Method as Verb;
use lemonfiber_ports::http::{Http, Method, Request};
use serde_json::json;

use super::{method, Sent, LARGEST};
use crate::fake::{json, raw, upstream};

/// A request for `path` at `fake`'s address.
fn request(address: &str, path: &str) -> Request {
    Request {
        method: Method::Get,
        url: format!("{address}{path}"),
        headers: vec![("x-asked".to_owned(), "yes".to_owned())],
        body: Some("{}".to_owned()),
        pinned: None,
    }
}

#[test]
fn each_method_is_sent_as_its_own_verb() {
    assert_eq!(method(Method::Get), reqwest::Method::GET);
    assert_eq!(method(Method::Post), reqwest::Method::POST);
    assert_eq!(method(Method::Put), reqwest::Method::PUT);
    assert_eq!(method(Method::Patch), reqwest::Method::PATCH);
    assert_eq!(method(Method::Delete), reqwest::Method::DELETE);
}

#[tokio::test]
async fn an_answer_is_read_whole_with_its_status_and_headers() {
    let fake = upstream(vec![json(Verb::GET, "/answered", &json!({ "one": 1 }))]).await;
    let sent = Sent(crate::serving::client().unwrap_or_default());

    let answered = sent.send(&request(&fake.address, "/answered")).await;

    let answered = answered.ok();
    assert_eq!(answered.as_ref().map(|one| one.status), Some(200));
    assert_eq!(
        answered.as_ref().map(|one| one.body.as_str()),
        Some("{\"one\":1}")
    );
    assert_eq!(
        answered
            .as_ref()
            .and_then(|one| one.header("content-type"))
            .map(str::to_owned),
        Some("application/json".to_owned())
    );
    let seen = fake.sent("GET", "/answered");
    assert_eq!(
        seen.first().and_then(|one| one.header("x-asked")),
        Some("yes")
    );
    assert_eq!(
        seen.first().and_then(|one| one.body.clone()),
        Some(json!({}))
    );
}

#[tokio::test]
async fn an_answer_larger_than_asked_for_is_not_read() {
    let fake = upstream(vec![raw(Verb::GET, "/large", Vec::new(), "0123456789")]).await;
    let sent = Sent(crate::serving::client().unwrap_or_default());

    let within = sent.fetch(&request(&fake.address, "/large"), 10).await;
    let past = sent.fetch(&request(&fake.address, "/large"), 9).await;

    assert_eq!(
        within.ok().and_then(|one| one.bytes),
        Some(b"0123456789".to_vec())
    );
    assert_eq!(past.ok().map(|one| one.bytes), Some(None));
}

#[tokio::test]
async fn an_answer_larger_than_the_contract_allows_is_not_sent_back() {
    let large: &'static str = Box::leak("x".repeat(LARGEST + 1).into_boxed_str());
    let fake = upstream(vec![raw(Verb::GET, "/larger", Vec::new(), large)]).await;
    let sent = Sent(crate::serving::client().unwrap_or_default());

    let answered = sent.send(&request(&fake.address, "/larger")).await;

    assert!(answered.is_err());
}

#[tokio::test]
async fn an_adapter_that_does_not_answer_is_unreachable() {
    let sent = Sent(crate::serving::client().unwrap_or_default());

    let answered = sent
        .fetch(&request("http://127.0.0.1:1", "/nothing"), LARGEST)
        .await;

    assert!(answered.is_err());
}
