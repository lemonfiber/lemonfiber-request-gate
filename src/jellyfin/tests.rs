use axum::body::Body;
use axum::http::{Method, StatusCode};
use base64::Engine;
use serde_json::{json, Value};

use crate::fake::{
    ask, body, json, raw, status, upstream, Answer, Answered, Config, Fake, JELLYFIN_TOKEN, KEY,
    RADARR_TOKEN, VERSION,
};

/// The device the request service signs its owner in on, as it sends it.
const OWNER: &str = "BOT_seerr";

/// A gate whose Jellyfin route reaches an upstream answering `answers`.
async fn gate(name: &str, answers: Vec<Answer>) -> (Config, Fake) {
    let fake = upstream(answers).await;
    let config = Config::new(name).with_routes(&fake.address);
    (config, fake)
}

/// `text` in base64.
fn encoded(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(text)
}

/// The authorisation the request service sends on `device`, with `token` where it
/// carries one.
fn seerr(device: &str, token: Option<&str>) -> String {
    let token = token
        .map(|token| format!(", Token=\"{token}\""))
        .unwrap_or_default();
    format!("MediaBrowser Client=\"Seerr\", Device=\"Seerr\", DeviceId=\"{device}\", Version=\"3.5.0\"{token}")
}

/// Ask `config`'s gate `method` on `path` of the Jellyfin route, with `authorisation`
/// and `sent` as the body.
async fn call(
    config: &Config,
    method: Method,
    path: &str,
    authorisation: Option<&str>,
    sent: Option<&Value>,
) -> Answered {
    let headers: Vec<(&str, &str)> = authorisation
        .map(|value| ("authorization", value))
        .into_iter()
        .collect();
    ask(
        config.service(),
        method,
        &format!("/jellyfin{path}"),
        &headers,
        sent.map_or_else(Body::empty, body),
    )
    .await
}

/// What `value` holds at `pointer`, or null.
fn at(value: &Value, pointer: &str) -> Value {
    value.pointer(pointer).cloned().unwrap_or_default()
}

/// Whether `value` reads as one that opens nothing: 64 hexadecimal digits.
fn opens_nothing(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[tokio::test]
async fn the_servers_name_and_an_avatar_go_upstream_with_no_credential() {
    let (config, fake) = gate(
        "public",
        vec![
            json(
                Method::GET,
                "/System/Info/Public",
                &json!({ "ServerName": "home", "Version": VERSION }),
            ),
            raw(
                Method::GET,
                "/UserImage",
                vec![
                    ("content-type", "image/png"),
                    ("etag", "\"abc\""),
                    ("last-modified", "Sat, 03 Oct 2026 10:00:00 GMT"),
                    ("x-other", "1"),
                ],
                "png",
            ),
            raw(Method::HEAD, "/UserImage", vec![("etag", "\"abc\"")], ""),
        ],
    )
    .await;
    let token = seerr(OWNER, Some(JELLYFIN_TOKEN));

    let name = call(
        &config,
        Method::GET,
        "/system/info/PUBLIC",
        Some(&token),
        None,
    )
    .await;
    let avatar = call(
        &config,
        Method::GET,
        "/UserImage?UserId=4f2a-9C&tag=1",
        None,
        None,
    )
    .await;
    let checked = call(&config, Method::HEAD, "/UserImage?UserId=4f2a", None, None).await;
    let bare = call(&config, Method::GET, "/UserImage", None, None).await;

    assert_eq!(
        name.json(),
        json!({ "ServerName": "home", "Version": VERSION })
    );
    assert_eq!(avatar.body, "png");
    for header in ["content-type", "etag", "last-modified"] {
        assert!(avatar.headers.contains_key(header), "{header}");
    }
    assert!(!avatar.headers.contains_key("x-other"));
    assert_eq!(checked.status, StatusCode::OK);
    assert_eq!(bare.status, StatusCode::OK);
    let seen = fake.seen();
    assert_eq!(
        seen.iter()
            .map(|one| one.query.as_deref())
            .collect::<Vec<_>>(),
        [Some("UserId=4f2a-9C"), Some("UserId=4f2a"), None]
    );
    assert!(fake
        .all()
        .iter()
        .all(|one| one.header("authorization").is_none()));
}

#[tokio::test]
async fn an_avatar_for_anything_but_one_account_is_refused() {
    let (config, fake) = gate("avatar-refused", Vec::new()).await;

    for path in [
        "/UserImage?UserId=../Users",
        "/UserImage?UserId=",
        "/UserImage?UserId=a&UserId=b",
    ] {
        let answered = call(&config, Method::GET, path, None, None).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn quick_connect_goes_upstream_on_the_request_services_device_and_nothing_else() {
    let (config, fake) = gate(
        "quick-connect",
        vec![
            json(
                Method::POST,
                "/QuickConnect/Initiate",
                &json!({ "Code": "123456", "Secret": "s" }),
            ),
            json(
                Method::GET,
                "/QuickConnect/Connect",
                &json!({ "Authenticated": false }),
            ),
        ],
    )
    .await;
    let device = encoded(OWNER);
    let authorisation = seerr(&device, Some("anything"));

    let initiated = call(
        &config,
        Method::POST,
        "/QuickConnect/Initiate",
        Some(&authorisation),
        Some(&json!({ "x": 1 })),
    )
    .await;
    let checked = call(
        &config,
        Method::GET,
        "/QuickConnect/Connect?secret=s&other=1",
        Some(&authorisation),
        None,
    )
    .await;

    assert_eq!(initiated.json(), json!({ "Code": "123456", "Secret": "s" }));
    assert_eq!(checked.json(), json!({ "Authenticated": false }));
    let seen = fake.seen();
    assert_eq!(
        seen.iter().map(|one| one.body.clone()).collect::<Vec<_>>(),
        [None, None]
    );
    assert_eq!(
        seen.last().and_then(|one| one.query.as_deref()),
        Some("secret=s")
    );
    let sent = format!(
        "MediaBrowser Client=\"Seerr\", Device=\"Seerr\", DeviceId=\"{device}\", Version=\"{}\"",
        env!("CARGO_PKG_VERSION")
    );
    assert!(seen
        .iter()
        .all(|one| one.header("authorization") == Some(sent.as_str())));
}

#[tokio::test]
async fn a_sign_in_on_a_device_the_request_service_does_not_assign_is_refused() {
    let (config, fake) = gate("devices-refused", Vec::new()).await;
    let on = |device: &str| seerr(device, None);

    for authorisation in [
        None,
        Some(on("someone")),
        Some(on(&encoded("BOT_seerr_"))),
        Some(on(&encoded("BOT_seerrx"))),
        Some(on("/w==")),
        Some("Bearer token".to_owned()),
    ] {
        let answered = call(
            &config,
            Method::POST,
            "/Users/AuthenticateByName",
            authorisation.as_deref(),
            Some(&json!({ "Username": "ana", "Pw": "secret" })),
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{authorisation:?}");
    }
    let quick = call(&config, Method::POST, "/QuickConnect/Initiate", None, None).await;

    assert_eq!(quick.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
}

/// A sign-in's answer for an account that `administers` the server.
fn signed_in(administers: bool) -> Value {
    json!({
        "User": { "Id": "u1", "Name": "ana", "ServerId": "s1", "Policy": { "IsAdministrator": administers } },
        "SessionInfo": { "Id": "session" },
        "AccessToken": "the-session-token",
        "ServerId": "s1"
    })
}

#[tokio::test]
async fn a_members_sign_in_goes_back_as_it_came() {
    let member = r#"{"User":{"Policy":{"IsAdministrator":false}},"AccessToken":"members-own"}"#;
    let (config, fake) = gate(
        "member",
        vec![raw(
            Method::POST,
            "/Users/AuthenticateByName",
            vec![("content-type", "application/json")],
            member,
        )],
    )
    .await;
    let device = encoded("BOT_seerr_ana");

    let answered = call(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        Some(&seerr(&device, None)),
        Some(&json!({ "Username": "ana", "Pw": "secret", "Extra": true })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    assert_eq!(answered.body, member);
    assert_eq!(
        answered
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(
        fake.sent("POST", "/Users/AuthenticateByName")
            .pop()
            .and_then(|seen| seen.body),
        Some(json!({ "Username": "ana", "Pw": "secret" }))
    );
    assert!(fake.sent("POST", "/Sessions/Logout").is_empty());
}

#[tokio::test]
async fn an_administrators_session_is_ended_and_its_token_replaced() {
    let (config, fake) = gate(
        "administrator",
        vec![
            json(Method::POST, "/Users/AuthenticateByName", &signed_in(true)),
            json(
                Method::POST,
                "/Users/AuthenticateWithQuickConnect",
                &signed_in(true),
            ),
            status(Method::POST, "/Sessions/Logout", StatusCode::NO_CONTENT),
        ],
    )
    .await;

    let by_name = call(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        Some(&seerr(OWNER, None)),
        Some(&json!({ "Username": "admin", "Pw": "the-password" })),
    )
    .await;
    let quick = call(
        &config,
        Method::POST,
        "/Users/AuthenticateWithQuickConnect",
        Some(&seerr(&encoded(OWNER), None)),
        Some(&json!({ "Secret": "s", "Other": 1 })),
    )
    .await;

    for answered in [&by_name, &quick] {
        assert_eq!(answered.status, StatusCode::OK);
        let read = answered.json();
        assert!(opens_nothing(&at(&read, "/AccessToken")), "{read}");
        assert_eq!(at(&read, "/User/Policy/IsAdministrator"), json!(true));
        assert_eq!(at(&read, "/User/Id"), json!("u1"));
        assert!(!answered.body.contains("the-session-token"));
    }
    assert_ne!(
        at(&by_name.json(), "/AccessToken"),
        at(&quick.json(), "/AccessToken")
    );
    assert_eq!(
        fake.sent("POST", "/Users/AuthenticateWithQuickConnect")
            .pop()
            .and_then(|seen| seen.body),
        Some(json!({ "Secret": "s" }))
    );
    let logouts = fake.sent("POST", "/Sessions/Logout");
    assert_eq!(logouts.len(), 2);
    assert!(logouts.iter().all(|one| one
        .header("authorization")
        .is_some_and(|value| value.contains("Token=\"the-session-token\""))));
}

#[tokio::test]
async fn an_administrators_session_the_gate_cannot_end_goes_back_to_nobody() {
    for logout in [
        status(Method::POST, "/Sessions/Logout", StatusCode::UNAUTHORIZED),
        status(Method::POST, "/Sessions/Unrelated", StatusCode::OK),
    ] {
        let (config, _) = gate(
            "not-ended",
            vec![
                json(Method::POST, "/Users/AuthenticateByName", &signed_in(true)),
                logout,
            ],
        )
        .await;

        let answered = call(
            &config,
            Method::POST,
            "/Users/AuthenticateByName",
            Some(&seerr(OWNER, None)),
            Some(&json!({ "Username": "admin", "Pw": "the-password" })),
        )
        .await;

        assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
        assert!(!answered.body.contains("the-session-token"));
    }
}

#[tokio::test]
async fn a_sign_in_answer_the_gate_cannot_read_goes_back_to_nobody() {
    let mut no_token = signed_in(true);
    if let Some(fields) = no_token.as_object_mut() {
        fields.remove("AccessToken");
    }
    for answer in [
        raw(
            Method::POST,
            "/Users/AuthenticateByName",
            Vec::new(),
            "not json",
        ),
        json(
            Method::POST,
            "/Users/AuthenticateByName",
            &json!({ "AccessToken": "t" }),
        ),
        json(Method::POST, "/Users/AuthenticateByName", &no_token),
    ] {
        let (config, fake) = gate("unreadable", vec![answer]).await;

        let answered = call(
            &config,
            Method::POST,
            "/Users/AuthenticateByName",
            Some(&seerr(OWNER, None)),
            Some(&json!({ "Username": "admin", "Pw": "the-password" })),
        )
        .await;

        assert_eq!(answered.status, StatusCode::BAD_GATEWAY);
        assert!(answered.body.is_empty());
        assert!(fake.sent("POST", "/Sessions/Logout").is_empty());
    }
}

#[tokio::test]
async fn a_failed_sign_in_goes_back_as_the_server_answered_it() {
    let (config, fake) = gate(
        "failed",
        vec![status(
            Method::POST,
            "/Users/AuthenticateByName",
            StatusCode::UNAUTHORIZED,
        )],
    )
    .await;

    let answered = call(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        Some(&seerr(OWNER, None)),
        Some(&json!({ "Username": "ana", "Pw": "wrong" })),
    )
    .await;
    let unread = call(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        Some(&seerr(OWNER, None)),
        None,
    )
    .await;
    let shaped = call(
        &config,
        Method::POST,
        "/Users/AuthenticateWithQuickConnect",
        Some(&seerr(OWNER, None)),
        Some(&json!({ "Secret": 7 })),
    )
    .await;
    let named = call(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        Some(&seerr(OWNER, None)),
        Some(&json!({ "Username": ["ana"], "Pw": "secret" })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::UNAUTHORIZED);
    assert_eq!(named.status, StatusCode::FORBIDDEN);
    assert_eq!(
        (unread.status, shaped.status),
        (StatusCode::FORBIDDEN, StatusCode::FORBIDDEN)
    );
    assert_eq!(fake.seen().len(), 1);
}

#[tokio::test]
async fn every_library_read_goes_upstream_under_the_gates_own_key() {
    let reads = [
        ("/System/Info", "/System/Info", None),
        ("/users", "/Users", None),
        ("/Users/Me", "/Users/me", None),
        ("/Users/4F2A/Views", "/Users/4f2a/Views", None),
        ("/Library/MediaFolders", "/Library/MediaFolders", None),
        (
            "/Items?SortBy=SortName&SortOrder=Ascending&IncludeItemTypes=Series,Movie,Others&Recursive=true&StartIndex=0&ParentId=p&collapseBoxSetItems=false&userId=other",
            "/Items",
            Some("SortBy=SortName&SortOrder=Ascending&IncludeItemTypes=Series%2CMovie%2COthers&Recursive=true&StartIndex=0&ParentId=p&collapseBoxSetItems=false"),
        ),
        ("/Items?ids=i1&fields=ProviderIds", "/Items", Some("ids=i1&fields=ProviderIds")),
        ("/Items/Latest?Limit=12&ParentId=p&userId=me&api_key=x", "/Items/Latest", Some("Limit=12&ParentId=p&userId=me")),
        ("/Shows/s1/Seasons?userId=x", "/Shows/s1/Seasons", None),
        ("/Shows/s1/Episodes?seasonId=n&fields=MediaSources&x=1", "/Shows/s1/Episodes", Some("seasonId=n&fields=MediaSources")),
    ];
    let answers = reads
        .iter()
        .map(|(_, path, _)| json(Method::GET, path, &json!({ "Items": [] })))
        .collect();
    let (config, fake) = gate("reads", answers).await;
    let authorisation = seerr("seerr", Some(JELLYFIN_TOKEN));
    let gates = format!(
        "MediaBrowser Client=\"lemonfiber-request-gate\", Device=\"lemonfiber-request-gate\", DeviceId=\"lemonfiber-request-gate\", Version=\"{}\", Token=\"{KEY}\"",
        env!("CARGO_PKG_VERSION")
    );

    for (path, upstream_path, query) in reads {
        let answered = call(&config, Method::GET, path, Some(&authorisation), None).await;
        assert_eq!(answered.status, StatusCode::OK, "{path}");
        let seen = fake.seen().pop();
        assert_eq!(
            seen.as_ref().map(|one| one.path.as_str()),
            Some(upstream_path),
            "{path}"
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.query.as_deref()),
            query,
            "{path}"
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.header("authorization")),
            Some(gates.as_str())
        );
    }
}

#[tokio::test]
async fn a_library_read_without_the_jellyfin_token_is_answered_401() {
    let (config, fake) = gate("reads-unauthorised", Vec::new()).await;

    for authorisation in [
        None,
        Some(seerr(OWNER, Some(RADARR_TOKEN))),
        Some(seerr(OWNER, None)),
    ] {
        let answered = call(
            &config,
            Method::GET,
            "/Users",
            authorisation.as_deref(),
            None,
        )
        .await;
        assert_eq!(
            answered.status,
            StatusCode::UNAUTHORIZED,
            "{authorisation:?}"
        );
    }
    assert!(fake.seen().is_empty());
    assert!(config.recorded().is_empty());
}

#[tokio::test]
async fn only_a_session_on_a_device_the_request_service_assigns_is_ended() {
    let (config, fake) = gate(
        "end-session",
        vec![status(Method::DELETE, "/Devices", StatusCode::NO_CONTENT)],
    )
    .await;
    let authorisation = seerr("seerr", Some(JELLYFIN_TOKEN));
    let member = encoded("BOT_seerr_ana");

    for id in [OWNER.to_owned(), encoded(OWNER), member.clone()] {
        let answered = call(
            &config,
            Method::DELETE,
            &format!("/Devices?Id={id}"),
            Some(&authorisation),
            None,
        )
        .await;
        assert_eq!(answered.status, StatusCode::NO_CONTENT, "{id}");
    }
    for query in [
        "Id=someone".to_owned(),
        format!("Id={}", encoded("BOT_seerr_")),
        format!("Id={}", encoded("another")),
        format!("Id={OWNER}&Id={member}"),
        String::new(),
    ] {
        let answered = call(
            &config,
            Method::DELETE,
            &format!("/Devices?{query}"),
            Some(&authorisation),
            None,
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{query}");
    }

    let sent: Vec<_> = fake
        .seen()
        .into_iter()
        .filter_map(|seen| seen.query)
        .collect();
    assert_eq!(sent.len(), 3);
    assert_eq!(sent.first().map(String::as_str), Some("Id=BOT_seerr"));
    assert_eq!(config.recorded().len(), 5);
}

#[tokio::test]
async fn the_key_mint_is_answered_by_the_gate_and_mints_nothing() {
    let (config, fake) = gate("keys", Vec::new()).await;
    let session = seerr(OWNER, Some("a-value-that-opens-nothing"));

    let minted = call(
        &config,
        Method::POST,
        "/Auth/Keys?App=Seerr",
        Some(&session),
        None,
    )
    .await;
    let listed = call(&config, Method::GET, "/Auth/Keys", Some(&session), None).await;
    let other = call(
        &config,
        Method::POST,
        "/Auth/Keys?App=Other",
        Some(&session),
        None,
    )
    .await;
    let bare = call(&config, Method::POST, "/Auth/Keys", None, None).await;

    assert_eq!(minted.status, StatusCode::NO_CONTENT);
    assert_eq!(listed.status, StatusCode::OK);
    let keys = listed.json();
    assert_eq!(at(&keys, "/Items").as_array().map(Vec::len), Some(1));
    assert_eq!(at(&keys, "/Items/0/AppName"), json!("Seerr"));
    assert!(opens_nothing(&at(&keys, "/Items/0/AccessToken")), "{keys}");
    assert_eq!(
        (other.status, bare.status),
        (StatusCode::FORBIDDEN, StatusCode::FORBIDDEN)
    );
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn administration_is_refused_and_recorded() {
    let (config, fake) = gate("administration", Vec::new()).await;
    let authorisation = seerr("seerr", Some(JELLYFIN_TOKEN));
    let refused = [
        (Method::POST, "/Users/New"),
        (Method::POST, "/Users/u1/Policy"),
        (Method::DELETE, "/Auth/Keys/abc"),
        (Method::GET, "/Auth/Keys/abc"),
        (Method::POST, "/Packages/Installed/plugin"),
        (Method::GET, "/System/Logs"),
        (Method::POST, "/System/Configuration"),
        (Method::GET, "/Users/a.b"),
        (Method::GET, "/Users/"),
        (Method::GET, "/Shows/a%2Fb/Seasons"),
        (Method::POST, "/Sessions/Logout"),
        (Method::DELETE, "/Users/u1"),
    ];

    for (method, path) in refused.clone() {
        let answered = call(&config, method.clone(), path, Some(&authorisation), None).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{method} {path}");
    }
    assert!(fake.seen().is_empty());
    assert_eq!(config.recorded().len(), refused.len());
}
