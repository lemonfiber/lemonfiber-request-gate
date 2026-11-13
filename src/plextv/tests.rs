use axum::body::Body;
use axum::http::{Method, StatusCode};
use lemonfiber_ports::service::{OutsideAccount, OutsideServer, PlexLink, PlexMember};
use serde_json::json;

use super::{addressed, escaped, id, users};
use crate::fake::{ask_on, json, status, upstream, Answered, Config, Fake};
use crate::serving::front_routes;

const GATE_TOKEN: &str = "the-gate-token";

fn account(id: &str, title: &str) -> OutsideAccount {
    OutsideAccount {
        id: id.to_owned(),
        uuid: format!("uuid-{id}"),
        username: format!("user{id}"),
        title: title.to_owned(),
    }
}

fn member(id: &str, name: &str) -> PlexMember {
    PlexMember {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

fn server() -> OutsideServer {
    OutsideServer {
        name: "Home & away".to_owned(),
        id: "the-machine".to_owned(),
    }
}

fn fronted() -> PlexLink {
    PlexLink::Fronted {
        account: account("77", "lemonfiber-request-gate"),
        members: vec![member("91", "Bo"), member("92", "Cy \"the kid\"")],
        server: server(),
    }
}

async fn plex() -> Fake {
    upstream(Vec::new()).await
}

/// plex.tv, answering a member's own sign-in.
async fn plex_tv() -> Fake {
    upstream(vec![json(
        Method::GET,
        "/users/account.json",
        &json!({
            "user": {
                "id": 91,
                "email": "Bo@Example.org",
                "username": "bo",
                "authToken": "bos-token",
                "profile": { "recoveryEmail": "bo@example.org" },
                "emails": ["bo@example.org", "other@example.net"],
            }
        }),
    )])
    .await
}

async fn front(
    config: &Config,
    accounts: &Fake,
    host: &str,
    path: &str,
    token: Option<&str>,
) -> Answered {
    let mut headers = vec![("Host", host)];
    if let Some(token) = token {
        headers.push(("X-Plex-Token", token));
    }
    ask_on(
        front_routes(config.reaching(&accounts.address)),
        Method::GET,
        path,
        &headers,
        Body::empty(),
    )
    .await
}

fn linked(fake: &Fake, link: PlexLink, name: &str) -> Config {
    Config::new(name).with_plex(fake, link, Some(GATE_TOKEN), Vec::new())
}

#[tokio::test]
async fn the_request_services_own_account_is_answered_with_the_token_it_signed_in_with_and_no_household_address(
) {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let config = linked(&fake, fronted(), "front-account");

    let answered = front(
        &config,
        &accounts,
        "plex.tv",
        "/users/account.json",
        Some(GATE_TOKEN),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let user = answered.json();
    assert_eq!(user.pointer("/user/id"), Some(&json!(77)));
    assert_eq!(user.pointer("/user/uuid"), Some(&json!("uuid-77")));
    assert_eq!(user.pointer("/user/email"), Some(&json!("77@plex.invalid")));
    assert_eq!(user.pointer("/user/authToken"), Some(&json!(GATE_TOKEN)));
    assert_eq!(
        user.pointer("/user/title"),
        Some(&json!("lemonfiber-request-gate"))
    );
    assert!(accounts.all().is_empty());
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn ping_users_and_resources_are_answered_from_what_the_core_applied() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let config = linked(&fake, fronted(), "front-calls");

    let ping = front(
        &config,
        &accounts,
        "plex.tv:443",
        "/api/v2/ping",
        Some(GATE_TOKEN),
    )
    .await;
    assert_eq!(ping.json(), json!({ "pong": true }));

    let users = front(
        &config,
        &accounts,
        "plex.tv",
        "/api/users",
        Some(GATE_TOKEN),
    )
    .await;
    assert_eq!(users.status, StatusCode::OK);
    assert!(
        users
            .body
            .contains(r#"<User id="91" title="Bo" username="Bo" email="91@plex.invalid""#),
        "{}",
        users.body
    );
    assert!(
        users.body.contains(r#"title="Cy &quot;the kid&quot;""#),
        "{}",
        users.body
    );
    assert!(
        users
            .body
            .contains(r#"machineIdentifier="the-machine" name="Home &amp; away""#),
        "{}",
        users.body
    );
    assert!(
        users.body.contains(r#"totalSize="2" size="2""#),
        "{}",
        users.body
    );
    assert!(!users.body.contains("example.org"));

    let resources = front(
        &config,
        &accounts,
        "PLEX.TV",
        "/api/resources?includeHttps=1",
        Some(GATE_TOKEN),
    )
    .await;
    assert_eq!(resources.status, StatusCode::OK);
    assert!(
        resources.body.contains(r#"clientIdentifier="the-machine""#),
        "{}",
        resources.body
    );
    assert!(
        resources.body.contains(r#"provides="server" owned="1""#),
        "{}",
        resources.body
    );
    assert_eq!(
        resources
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/xml; charset=utf-8")
    );
    assert!(accounts.all().is_empty());
}

#[tokio::test]
async fn a_members_own_sign_in_is_passed_on_to_plex_tv_and_its_answer_handed_back() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let config = linked(&fake, fronted(), "front-member");

    let answered = front(
        &config,
        &accounts,
        "plex.tv",
        "/users/account.json",
        Some("bos-token"),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let user = answered.json();
    assert_eq!(user.pointer("/user/id"), Some(&json!(91)));
    assert_eq!(user.pointer("/user/username"), Some(&json!("bo")));
    assert_eq!(user.pointer("/user/authToken"), Some(&json!("bos-token")));
    assert_eq!(user.pointer("/user/email"), Some(&json!("91@plex.invalid")));
    assert_eq!(
        user.pointer("/user/profile/recoveryEmail"),
        Some(&json!("91@plex.invalid"))
    );
    assert_eq!(
        user.pointer("/user/emails"),
        Some(&json!(["91@plex.invalid", "other@example.net"]))
    );
    assert!(!answered.body.to_ascii_lowercase().contains("example.org"));
    let passed = accounts.sent("GET", "/users/account.json");
    assert_eq!(passed.len(), 1);
    assert_eq!(
        passed.first().and_then(|one| one.header("x-plex-token")),
        Some("bos-token")
    );

    let unreachable = Config::new("front-member-unreachable").with_plex(
        &fake,
        fronted(),
        Some(GATE_TOKEN),
        Vec::new(),
    );
    let answered = ask_on(
        front_routes(unreachable.reaching("http://127.0.0.1:1")),
        Method::GET,
        "/users/account.json",
        &[("Host", "plex.tv"), ("X-Plex-Token", "bos-token")],
        Body::empty(),
    )
    .await;
    assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn every_other_call_is_refused_recorded_and_sent_nowhere() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let config = linked(&fake, fronted(), "front-refused");

    for (host, path, token) in [
        ("plex.tv", "/api/users", Some("bos-token")),
        ("plex.tv", "/api/v2/user", Some(GATE_TOKEN)),
        (
            "discover.provider.plex.tv",
            "/library/sections/watchlist/all",
            Some(GATE_TOKEN),
        ),
        ("plex.tv", "/users/account.json/x", Some(GATE_TOKEN)),
    ] {
        let answered = front(&config, &accounts, host, path, token).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{host}{path}");
    }
    let posted = ask_on(
        front_routes(config.reaching(&accounts.address)),
        Method::POST,
        "/users/account.json",
        &[("Host", "plex.tv"), ("X-Plex-Token", GATE_TOKEN)],
        Body::empty(),
    )
    .await;
    assert_eq!(posted.status, StatusCode::FORBIDDEN);
    let hostless = ask_on(
        front_routes(config.reaching(&accounts.address)),
        Method::GET,
        "/api/v2/ping",
        &[("X-Plex-Token", GATE_TOKEN)],
        Body::empty(),
    )
    .await;
    assert_eq!(hostless.status, StatusCode::FORBIDDEN);
    assert_eq!(config.recorded().len(), 6);
    assert!(accounts.all().is_empty());
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn a_call_with_no_token_is_unauthorised() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let config = linked(&fake, fronted(), "front-tokenless");

    let answered = front(&config, &accounts, "plex.tv", "/api/v2/ping", None).await;

    assert_eq!(answered.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn outside_fronted_mode_or_before_a_plex_route_the_front_answers_nothing() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    for (link, name) in [
        (PlexLink::Managed, "front-managed"),
        (
            PlexLink::Owner {
                account: "4242".to_owned(),
            },
            "front-owner",
        ),
    ] {
        let config = linked(&fake, link, name);
        for (path, token) in [
            ("/users/account.json", GATE_TOKEN),
            ("/users/account.json", "bos-token"),
            ("/api/v2/ping", GATE_TOKEN),
        ] {
            let answered = front(&config, &accounts, "plex.tv", path, Some(token)).await;
            assert_eq!(
                answered.status,
                StatusCode::FORBIDDEN,
                "{name} {path} {token}"
            );
        }
    }
    let unwritten = Config::new("front-unwritten");
    assert_eq!(
        front(
            &unwritten,
            &accounts,
            "plex.tv",
            "/api/v2/ping",
            Some(GATE_TOKEN)
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let elsewhere = Config::new("front-elsewhere").with_upstream(&fake);
    assert_eq!(
        front(
            &elsewhere,
            &accounts,
            "plex.tv",
            "/api/v2/ping",
            Some(GATE_TOKEN)
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert!(accounts.all().is_empty());
}

#[tokio::test]
async fn an_account_whose_id_is_no_number_is_not_answered() {
    let fake = plex().await;
    let accounts = plex_tv().await;
    let broken = PlexLink::Fronted {
        account: account("seventy", "x"),
        members: vec![member("nine", "y")],
        server: server(),
    };
    let config = linked(&fake, broken, "front-broken");

    for path in ["/users/account.json", "/api/users"] {
        let answered = front(&config, &accounts, "plex.tv", path, Some(GATE_TOKEN)).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(users(&[member("nine", "y")], &server()).is_none());
}

#[test]
fn an_id_is_digits_whether_plex_tv_writes_a_number_or_a_string() {
    assert_eq!(id(&json!(42)), Some("42".to_owned()));
    assert_eq!(id(&json!("42")), Some("42".to_owned()));
    for not_one in [
        json!(-1),
        json!(1.5),
        json!(""),
        json!("4a"),
        json!(null),
        json!([1]),
    ] {
        assert_eq!(id(&not_one), None, "{not_one}");
    }
}

#[test]
fn an_attribute_value_escapes_every_mark_xml_reads_otherwise() {
    assert_eq!(escaped(r#"a&b<c>d"e'f"#), "a&amp;b&lt;c&gt;d&quot;e&apos;f");
}

#[tokio::test]
async fn whose_each_token_is_is_kept_for_a_bounded_number_of_tokens_then_asked_again() {
    let accounts = upstream(vec![json(
        Method::GET,
        "/users/account.json",
        &json!({ "user": { "id": 4242 } }),
    )])
    .await;
    let plex_tv = super::PlexTv::at(&accounts.address);
    let client = crate::serving::client().unwrap_or_default();

    for at in 0..=super::KEPT {
        let owner = plex_tv.owner(&client, &format!("token-{at}")).await;
        assert_eq!(owner.ok().flatten().as_deref(), Some("4242"));
    }
    let first_again = plex_tv.owner(&client, "token-0").await;

    assert_eq!(first_again.ok().flatten().as_deref(), Some("4242"));
    assert_eq!(
        accounts.sent("GET", "/users/account.json").len(),
        super::KEPT + 2
    );
}

#[tokio::test]
async fn a_members_refusal_is_handed_back_as_plex_tv_gave_it_and_an_account_without_an_id_is_not() {
    let fake = plex().await;
    let refusing = upstream(vec![status(
        Method::GET,
        "/users/account.json",
        StatusCode::UNAUTHORIZED,
    )])
    .await;
    let config = linked(&fake, fronted(), "front-member-refused");

    let refused = front(
        &config,
        &refusing,
        "plex.tv",
        "/users/account.json",
        Some("stale-token"),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refusing.sent("GET", "/users/account.json").len(), 1);

    let idless = upstream(vec![json(
        Method::GET,
        "/users/account.json",
        &json!({ "user": { "email": "bo@example.org" } }),
    )])
    .await;
    let unread = front(
        &config,
        &idless,
        "plex.tv",
        "/users/account.json",
        Some("bos-token"),
    )
    .await;
    assert_eq!(unread.status, StatusCode::BAD_GATEWAY);
    assert!(!unread.body.contains("example.org"));

    let unreadable = upstream(vec![crate::fake::raw(
        Method::GET,
        "/users/account.json",
        vec![("content-type", "text/html")],
        "<p>bo@example.org</p>",
    )])
    .await;
    let garbled = front(
        &config,
        &unreadable,
        "plex.tv",
        "/users/account.json",
        Some("bos-token"),
    )
    .await;
    assert_eq!(garbled.status, StatusCode::BAD_GATEWAY);
    assert!(!garbled.body.contains("example.org"));
}

#[test]
fn an_account_without_an_address_is_given_one_and_one_named_by_no_id_is_none() {
    assert_eq!(
        addressed(json!({ "user": { "id": "7" } })),
        Some(json!({ "user": { "id": "7", "email": "7@plex.invalid" } }))
    );
    assert_eq!(
        addressed(json!({ "user": { "id": "x", "email": "a@b" } })),
        None
    );
    assert_eq!(addressed(json!([{ "user": { "id": 7 } }])), None);
}
