use axum::body::Body;
use axum::http::{Method, StatusCode};
use lemonfiber_ports::service::{OutsideAccount, OutsideServer, PlexLink};
use serde_json::json;

use crate::fake::{ask_on, json, status, upstream, Config, Fake, KEY, PLEX_TOKEN};
use crate::serving::plex_routes;

const STATUS: &str = "/";

fn status_of(version: &str) -> serde_json::Value {
    json!({ "MediaContainer": { "machineIdentifier": "the-machine", "friendlyName": "home", "version": version } })
}

/// A Plex answering its status, its libraries, a library's contents, an item and its
/// children.
async fn plex() -> Fake {
    upstream(vec![
        json(Method::GET, STATUS, &status_of("1.43.4.10903-e5521bd8c")),
        json(
            Method::GET,
            "/library/sections",
            &json!({ "MediaContainer": { "Directory": [{ "key": "1", "type": "movie" }] } }),
        ),
        json(
            Method::GET,
            "/library/sections/1/all",
            &json!({ "MediaContainer": { "totalSize": 0, "Metadata": [] } }),
        ),
        json(
            Method::GET,
            "/library/metadata/42",
            &json!({ "MediaContainer": { "Metadata": [{ "ratingKey": "42" }] } }),
        ),
        json(
            Method::GET,
            "/library/metadata/42/children",
            &json!({ "MediaContainer": { "Metadata": [] } }),
        ),
    ])
    .await
}

fn managed(fake: &Fake, name: &str) -> Config {
    Config::new(name).with_plex(fake, PlexLink::Managed, Some(PLEX_TOKEN), Vec::new())
}

async fn asked(config: &Config, method: Method, uri: &str, headers: &[(&str, &str)]) -> StatusCode {
    ask_on(
        plex_routes(config.service()),
        method,
        uri,
        headers,
        Body::empty(),
    )
    .await
    .status
}

#[tokio::test]
async fn each_read_on_the_list_goes_to_plex_as_the_request_services_own_account() {
    let fake = plex().await;
    let config = managed(&fake, "plex-reads");
    let presented = [("X-Plex-Token", PLEX_TOKEN)];

    for path in [
        "/",
        "/library/sections",
        "/library/sections/1/all",
        "/library/metadata/42",
        "/library/metadata/42/children",
    ] {
        assert_eq!(
            asked(&config, Method::GET, path, &presented).await,
            StatusCode::OK,
            "{path}"
        );
    }
    let seen = fake.all();
    assert!(seen
        .iter()
        .all(|one| one.header("x-plex-token") == Some(KEY)));
    assert!(seen
        .iter()
        .all(|one| one.header("accept") == Some("application/json")));
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn a_listing_carries_only_the_parameters_and_paging_the_list_names() {
    let fake = plex().await;
    let config = managed(&fake, "plex-listing");

    let answered = asked(
        &config,
        Method::GET,
        "/library/sections/1/all?type=1&includeGuids=1&sort=addedAt%3Adesc&addedAt%3E%3E=1700000000&X-Plex-Token=the-plex-token&other=x",
        &[
            ("X-Plex-Container-Start", "0"),
            ("X-Plex-Container-Size", "500"),
            ("X-Plex-Client-Identifier", "seerr"),
        ],
    )
    .await;

    assert_eq!(answered, StatusCode::OK);
    let sent = fake.sent("GET", "/library/sections/1/all");
    assert_eq!(sent.len(), 1);
    let header = |name: &str| sent.first().and_then(|one| one.header(name));
    let query = sent
        .first()
        .and_then(|one| one.query.clone())
        .unwrap_or_default();
    for wanted in [
        "type=1",
        "includeGuids=1",
        "sort=addedAt%3Adesc",
        "addedAt%3E%3E=1700000000",
    ] {
        assert!(query.contains(wanted), "{wanted} in {query}");
    }
    assert!(
        !query.contains("other") && !query.contains("plex-token"),
        "{query}"
    );
    assert_eq!(header("x-plex-container-start"), Some("0"));
    assert_eq!(header("x-plex-container-size"), Some("500"));
    assert_eq!(header("x-plex-client-identifier"), None);
}

#[tokio::test]
async fn an_item_is_read_with_its_children_where_asked() {
    let fake = plex().await;
    let config = managed(&fake, "plex-children");

    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/metadata/42?includeChildren=1",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::OK
    );
    let sent = fake.sent("GET", "/library/metadata/42");
    assert_eq!(
        sent.first().and_then(|one| one.query.clone()).as_deref(),
        Some("includeChildren=1")
    );
}

#[tokio::test]
async fn a_call_off_the_list_is_refused_recorded_and_sent_nowhere() {
    let fake = plex().await;
    let config = managed(&fake, "plex-refused");
    let presented = [("X-Plex-Token", PLEX_TOKEN)];

    for (method, path) in [
        (Method::POST, "/library/sections"),
        (Method::GET, "/library/sections/1/refresh"),
        (Method::GET, "/library/metadata/4a"),
        (Method::GET, "/library/metadata/123456789012345678901"),
        (Method::GET, "/:/prefs"),
        (Method::GET, "/library/sections/"),
    ] {
        assert_eq!(
            asked(&config, method.clone(), path, &presented).await,
            StatusCode::FORBIDDEN,
            "{method} {path}"
        );
    }
    assert!(fake.all().is_empty());
    assert_eq!(config.recorded().len(), 6);
}

#[tokio::test]
async fn a_parameter_or_page_in_another_shape_is_refused_and_sent_nowhere() {
    let fake = plex().await;
    let config = managed(&fake, "plex-shapes");
    let presented = ("X-Plex-Token", PLEX_TOKEN);

    for (uri, page) in [
        ("/library/sections/1/all?type=1%261", None),
        ("/library/sections/1/all?type=1&type=2", None),
        ("/library/sections/1/all", Some("ten")),
        ("/library/sections/1/all", Some("")),
    ] {
        let mut headers = vec![presented];
        if let Some(page) = page {
            headers.push(("X-Plex-Container-Size", page));
        }
        assert_eq!(
            asked(&config, Method::GET, uri, &headers).await,
            StatusCode::FORBIDDEN,
            "{uri} {page:?}"
        );
    }
    assert!(fake.sent("GET", "/library/sections/1/all").is_empty());
}

#[tokio::test]
async fn a_token_the_route_does_not_admit_or_two_tokens_that_differ_are_unauthorised() {
    let fake = plex().await;
    let config = managed(&fake, "plex-tokens");

    for (uri, headers) in [
        ("/library/sections", vec![]),
        ("/library/sections", vec![("X-Plex-Token", "another-token")]),
        (
            "/library/sections?X-Plex-Token=another-token",
            vec![("X-Plex-Token", PLEX_TOKEN)],
        ),
    ] {
        assert_eq!(
            asked(&config, Method::GET, uri, &headers).await,
            StatusCode::UNAUTHORIZED,
            "{uri} {headers:?}"
        );
    }
    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/sections?X-Plex-Token=the-plex-token",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/sections?X-Plex-Token=the-plex-token",
            &[]
        )
        .await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn nothing_is_answered_before_the_core_writes_a_plex_route() {
    let fake = plex().await;
    let unwritten = Config::new("plex-unwritten");
    assert_eq!(
        asked(
            &unwritten,
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let elsewhere = Config::new("plex-elsewhere").with_upstream(&fake);
    assert_eq!(
        asked(
            &elsewhere,
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::FORBIDDEN
    );
    let unreadable = Config::new("plex-unreadable");
    unreadable.write(crate::formats::gate::File::Upstreams, "not the routes");
    assert_eq!(
        asked(&unreadable, Method::GET, "/library/sections", &[]).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        asked(&unwritten, Method::GET, "/health", &[]).await,
        StatusCode::OK
    );
    assert!(fake.all().is_empty());
}

#[tokio::test]
async fn in_fronted_mode_the_route_admits_the_gate_token() {
    let fake = plex().await;
    let config = Config::new("plex-fronted").with_plex(
        &fake,
        PlexLink::Fronted {
            account: OutsideAccount {
                id: "77".to_owned(),
                uuid: "u".to_owned(),
                username: String::new(),
                title: "lemonfiber-request-gate".to_owned(),
            },
            members: Vec::new(),
            server: OutsideServer {
                name: "home".to_owned(),
                id: "the-machine".to_owned(),
            },
        },
        Some("the-gate-token"),
        Vec::new(),
    );

    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", "the-gate-token")]
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        fake.sent("GET", "/library/sections")
            .first()
            .and_then(|one| one.header("x-plex-token").map(str::to_owned))
            .as_deref(),
        Some(KEY)
    );
}

/// plex.tv, answering whose a token is with the account `id`, or refusing it.
async fn plex_tv(answer: Option<u64>) -> Fake {
    upstream(vec![match answer {
        Some(id) => json(
            Method::GET,
            "/users/account.json",
            &json!({ "user": { "id": id, "uuid": "u", "email": "e", "username": "o", "title": "Owner" } }),
        ),
        None => status(Method::GET, "/users/account.json", StatusCode::UNAUTHORIZED),
    }])
    .await
}

fn owned(fake: &Fake, name: &str) -> Config {
    Config::new(name).with_plex(
        fake,
        PlexLink::Owner {
            account: "4242".to_owned(),
        },
        None,
        Vec::new(),
    )
}

#[tokio::test]
async fn in_owner_mode_a_token_plex_tv_answers_as_the_claiming_account_is_admitted_asked_once() {
    let fake = plex().await;
    let accounts = plex_tv(Some(4242)).await;
    let config = owned(&fake, "plex-owner");
    let service = config.reaching(&accounts.address);

    for _ in 0..2 {
        let answered = ask_on(
            plex_routes(std::sync::Arc::clone(&service)),
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", "the-owners-token")],
            Body::empty(),
        )
        .await;
        assert_eq!(answered.status, StatusCode::OK);
    }
    let checked = accounts.sent("GET", "/users/account.json");
    assert_eq!(checked.len(), 1);
    assert_eq!(
        checked.first().and_then(|one| one.header("x-plex-token")),
        Some("the-owners-token")
    );
    assert!(fake
        .all()
        .iter()
        .all(|one| one.header("x-plex-token") == Some(KEY)));
}

#[tokio::test]
async fn in_owner_mode_another_account_a_refused_token_or_plex_tv_failing_is_unauthorised() {
    let fake = plex().await;
    for (answer, name) in [(Some(7), "plex-owner-other"), (None, "plex-owner-refused")] {
        let accounts = plex_tv(answer).await;
        let config = owned(&fake, name);
        let answered = ask_on(
            plex_routes(config.reaching(&accounts.address)),
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", "a-token")],
            Body::empty(),
        )
        .await;
        assert_eq!(answered.status, StatusCode::UNAUTHORIZED, "{answer:?}");
    }
    let failing = upstream(vec![status(
        Method::GET,
        "/users/account.json",
        StatusCode::SERVICE_UNAVAILABLE,
    )])
    .await;
    let config = owned(&fake, "plex-owner-failing");
    assert_eq!(
        ask_on(
            plex_routes(config.reaching(&failing.address)),
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", "a-token")],
            Body::empty(),
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    let unread = upstream(vec![json(
        Method::GET,
        "/users/account.json",
        &json!({ "user": {} }),
    )])
    .await;
    let config = owned(&fake, "plex-owner-unread");
    assert_eq!(
        ask_on(
            plex_routes(config.reaching(&unread.address)),
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", "a-token")],
            Body::empty(),
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(fake.sent("GET", "/library/sections").is_empty());
}

#[tokio::test]
async fn a_plex_on_a_major_the_route_does_not_name_is_answered_unavailable_naming_it() {
    let fake = upstream(vec![json(Method::GET, STATUS, &status_of("2.0.1"))]).await;
    let config =
        Config::new("plex-major").with_plex(&fake, PlexLink::Managed, Some(PLEX_TOKEN), vec![1]);

    let answered = ask_on(
        plex_routes(config.service()),
        Method::GET,
        "/library/sections",
        &[("X-Plex-Token", PLEX_TOKEN)],
        Body::empty(),
    )
    .await;

    assert_eq!(answered.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        answered.body,
        "Plex 2.0.1 is not a version this gate supports"
    );
    assert!(fake.sent("GET", "/library/sections").is_empty());

    let supported = plex().await;
    let config = Config::new("plex-major-supported").with_plex(
        &supported,
        PlexLink::Managed,
        Some(PLEX_TOKEN),
        vec![1],
    );
    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::OK
    );
    let unread = upstream(vec![json(
        Method::GET,
        STATUS,
        &json!({ "MediaContainer": {} }),
    )])
    .await;
    let config = Config::new("plex-major-unread").with_plex(
        &unread,
        PlexLink::Managed,
        Some(PLEX_TOKEN),
        vec![1],
    );
    assert_eq!(
        asked(
            &config,
            Method::GET,
            "/library/sections",
            &[("X-Plex-Token", PLEX_TOKEN)]
        )
        .await,
        StatusCode::BAD_GATEWAY
    );
}
