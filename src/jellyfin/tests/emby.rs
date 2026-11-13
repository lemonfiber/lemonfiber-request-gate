use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use super::{at, encoded, opens_nothing, seerr, signed_in, OWNER};
use crate::fake::{
    ask, body, json, raw, status, upstream, Answer, Answered, Config, Fake, EMBY_ROUTE,
    JELLYFIN_TOKEN, KEY, MOVIES_TOKEN,
};

/// A gate whose media server's route is an Emby route reaching an upstream answering
/// `answers`.
async fn gate(name: &str, answers: Vec<Answer>) -> (Config, Fake) {
    let fake = upstream(answers).await;
    let config = Config::new(name).with_emby(&fake);
    (config, fake)
}

/// Ask `config`'s gate `method` on `path` of the Emby route, with `authorisation` and
/// `sent` as the body.
async fn on_emby(
    config: &Config,
    method: Method,
    path: &str,
    authorisation: &str,
    sent: Option<&Value>,
) -> Answered {
    ask(
        config.service(),
        method,
        &format!("/{EMBY_ROUTE}{path}"),
        &[("authorization", authorisation)],
        sent.map_or_else(Body::empty, body),
    )
    .await
}

/// The authorisation Emby reads from the client `client` on the device `device`, as the
/// gate sends it.
fn embys(client: &str, device: &str) -> String {
    format!(
        "Emby Client=\"{client}\", Device=\"{client}\", DeviceId=\"{device}\", Version=\"{}\"",
        env!("CARGO_PKG_VERSION")
    )
}

#[tokio::test]
async fn emby_is_answered_on_the_jellyfin_list_in_its_own_api() {
    let (config, fake) = gate(
        "emby",
        vec![
            json(Method::GET, "/Users", &json!([{ "Id": "a1" }])),
            json(Method::GET, "/Users/a1/Items/Latest", &json!([])),
            raw(
                Method::GET,
                "/Users/a1/Images/Primary",
                vec![("content-type", "image/png")],
                "png",
            ),
            raw(
                Method::HEAD,
                "/Users/a1/Images/Primary",
                vec![("etag", "\"v7\"")],
                "",
            ),
        ],
    )
    .await;
    let token = seerr(OWNER, Some(JELLYFIN_TOKEN));

    let users = on_emby(&config, Method::GET, "/Users", &token, None).await;
    let latest = on_emby(
        &config,
        Method::GET,
        "/Users/a1/Items/Latest?Limit=12&ParentId=p1&userId=a1",
        &token,
        None,
    )
    .await;
    let avatar = on_emby(
        &config,
        Method::GET,
        "/Users/a1/Images/Primary?quality=90",
        "",
        None,
    )
    .await;
    let checked = on_emby(&config, Method::HEAD, "/Users/a1/Images/Primary", "", None).await;

    assert_eq!(users.json(), json!([{ "Id": "a1" }]));
    assert_eq!(latest.status, StatusCode::OK);
    assert_eq!(
        (avatar.status, avatar.body.as_str()),
        (StatusCode::OK, "png")
    );
    assert_eq!(checked.status, StatusCode::OK);
    assert_eq!(
        checked
            .headers
            .get("etag")
            .and_then(|value| value.to_str().ok()),
        Some("\"v7\"")
    );
    let sent_latest = fake.sent("GET", "/Users/a1/Items/Latest");
    assert_eq!(
        sent_latest.first().and_then(|one| one.query.clone()),
        Some("Limit=12&ParentId=p1".to_owned())
    );
    assert_eq!(
        sent_latest
            .first()
            .and_then(|one| one.header("x-emby-token")),
        Some(KEY)
    );
    let sent_avatar = fake.sent("GET", "/Users/a1/Images/Primary");
    assert_eq!(
        sent_avatar.first().and_then(|one| one.query.clone()),
        Some("quality=90".to_owned())
    );
    assert!(fake
        .all()
        .iter()
        .all(|one| one.header("authorization").is_none()));
    assert!(sent_avatar
        .iter()
        .all(|one| one.header("x-emby-token").is_none()
            && one.header("x-emby-authorization").is_none()));
}

#[tokio::test]
async fn each_media_server_is_answered_only_in_its_own_api() {
    let fake = upstream(vec![
        json(Method::GET, "/Items/Latest", &json!([])),
        raw(Method::GET, "/UserImage", Vec::new(), "png"),
        json(Method::GET, "/Users/a1/Items/Latest", &json!([])),
        raw(Method::GET, "/Users/a1/Images/Primary", Vec::new(), "png"),
    ])
    .await;
    let emby = Config::new("emby-own-api").with_emby(&fake);
    let jellyfin = Config::new("jellyfin-own-api").with_upstream(&fake);
    let token = seerr(OWNER, Some(JELLYFIN_TOKEN));

    for path in [
        "/Items/Latest",
        "/UserImage?UserId=a1",
        "/Users/a!/Images/Primary",
    ] {
        let answered = on_emby(&emby, Method::GET, path, &token, None).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    for path in ["/Users/a1/Items/Latest", "/Users/a1/Images/Primary"] {
        let answered = ask(
            jellyfin.service(),
            Method::GET,
            &format!("/jellyfin{path}"),
            &[("authorization", token.as_str())],
            Body::empty(),
        )
        .await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{path}");
    }
    let unshaped = on_emby(
        &emby,
        Method::GET,
        "/Users/a1/Images/Primary?quality=high",
        "",
        None,
    )
    .await;

    assert_eq!(unshaped.status, StatusCode::FORBIDDEN);
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn quick_connect_which_emby_has_not_is_refused_on_its_route_and_recorded() {
    let (config, fake) = gate("emby-quick-connect", Vec::new()).await;
    let device = seerr(&encoded(OWNER), None);
    let calls = [
        (Method::POST, "/QuickConnect/Initiate", None),
        (Method::GET, "/QuickConnect/Connect?secret=s", None),
        (
            Method::POST,
            "/Users/AuthenticateWithQuickConnect",
            Some(json!({ "Secret": "s" })),
        ),
    ];

    for (method, path, sent) in calls.clone() {
        let answered = on_emby(&config, method.clone(), path, &device, sent.as_ref()).await;
        assert_eq!(answered.status, StatusCode::FORBIDDEN, "{method} {path}");
    }

    assert!(fake.seen().is_empty());
    assert_eq!(
        config.recorded(),
        [
            "POST /emby/QuickConnect/Initiate",
            "GET /emby/QuickConnect/Connect",
            "POST /emby/Users/AuthenticateWithQuickConnect",
        ]
    );
}

#[tokio::test]
async fn every_library_read_goes_to_emby_under_the_gates_own_key_in_embys_headers() {
    let reads = [
        ("/System/Info", "/System/Info", None),
        ("/users", "/Users", None),
        ("/Users/Me", "/Users/me", None),
        ("/Users/4F2A/Views", "/Users/4f2a/Views", None),
        ("/Library/MediaFolders", "/Library/MediaFolders", None),
        (
            "/Items?SortBy=SortName&SortOrder=Ascending&IncludeItemTypes=Series,Movie,Others&Recursive=true&StartIndex=0&ParentId=p&collapseBoxSetItems=false&api_key=x",
            "/Items",
            Some("SortBy=SortName&SortOrder=Ascending&IncludeItemTypes=Series%2CMovie%2COthers&Recursive=true&StartIndex=0&ParentId=p&collapseBoxSetItems=false"),
        ),
        ("/Items?ids=i1&fields=ProviderIds", "/Items", Some("ids=i1&fields=ProviderIds")),
        ("/Users/a1/Items/Latest?Limit=12&ParentId=p", "/Users/a1/Items/Latest", Some("Limit=12&ParentId=p")),
        ("/Shows/s1/Seasons?userId=x", "/Shows/s1/Seasons", None),
        ("/Shows/s1/Episodes?seasonId=n&fields=MediaSources&x=1", "/Shows/s1/Episodes", Some("seasonId=n&fields=MediaSources")),
    ];
    let answers = reads
        .iter()
        .map(|(_, path, _)| json(Method::GET, path, &json!({ "Items": [] })))
        .collect();
    let (config, fake) = gate("emby-reads", answers).await;
    let authorisation = seerr("seerr", Some(JELLYFIN_TOKEN));
    let gates = embys("lemonfiber-request-gate", "lemonfiber-request-gate");

    for (path, upstream_path, query) in reads {
        let answered = on_emby(&config, Method::GET, path, &authorisation, None).await;
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
            seen.as_ref()
                .and_then(|one| one.header("x-emby-authorization")),
            Some(gates.as_str()),
            "{path}"
        );
        assert_eq!(
            seen.as_ref().and_then(|one| one.header("x-emby-token")),
            Some(KEY),
            "{path}"
        );
        assert!(seen
            .as_ref()
            .is_some_and(|one| one.header("authorization").is_none()));
    }
}

#[tokio::test]
async fn a_read_without_the_emby_routes_token_is_answered_401() {
    let (config, fake) = gate("emby-unauthorised", Vec::new()).await;

    for authorisation in [
        String::new(),
        seerr(OWNER, Some(MOVIES_TOKEN)),
        seerr(OWNER, None),
    ] {
        let answered = on_emby(&config, Method::GET, "/Users", &authorisation, None).await;
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
async fn an_administrators_emby_session_is_ended_in_embys_headers_and_its_token_replaced() {
    let (config, fake) = gate(
        "emby-administrator",
        vec![
            json(Method::POST, "/Users/AuthenticateByName", &signed_in(true)),
            status(Method::POST, "/Sessions/Logout", StatusCode::NO_CONTENT),
        ],
    )
    .await;

    let answered = on_emby(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        &seerr(OWNER, None),
        Some(&json!({ "Username": "admin", "Pw": "the-password", "Extra": 1 })),
    )
    .await;

    assert_eq!(answered.status, StatusCode::OK);
    let read = answered.json();
    assert!(opens_nothing(&at(&read, "/AccessToken")), "{read}");
    assert_eq!(at(&read, "/User/Policy/IsAdministrator"), json!(true));
    assert!(!answered.body.contains("the-session-token"));
    let signed = fake.sent("POST", "/Users/AuthenticateByName");
    assert_eq!(
        signed.first().and_then(|one| one.body.clone()),
        Some(json!({ "Username": "admin", "Pw": "the-password" }))
    );
    let device = embys("Seerr", OWNER);
    assert_eq!(
        signed
            .first()
            .and_then(|one| one.header("x-emby-authorization")),
        Some(device.as_str())
    );
    assert!(signed
        .first()
        .is_some_and(|one| one.header("x-emby-token").is_none()));
    let logouts = fake.sent("POST", "/Sessions/Logout");
    assert_eq!(logouts.len(), 1);
    assert_eq!(
        logouts.first().and_then(|one| one.header("x-emby-token")),
        Some("the-session-token")
    );
    assert_eq!(
        logouts
            .first()
            .and_then(|one| one.header("x-emby-authorization")),
        Some(device.as_str())
    );
    assert!(fake
        .all()
        .iter()
        .all(|one| one.header("authorization").is_none()));
}

#[tokio::test]
async fn a_members_emby_sign_in_goes_back_as_it_came_and_another_device_is_refused() {
    let member = r#"{"User":{"Policy":{"IsAdministrator":false}},"AccessToken":"members-own"}"#;
    let (config, fake) = gate(
        "emby-member",
        vec![raw(
            Method::POST,
            "/Users/AuthenticateByName",
            vec![("content-type", "application/json")],
            member,
        )],
    )
    .await;
    let sent = json!({ "Username": "ana", "Pw": "secret" });

    let answered = on_emby(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        &seerr(&encoded("BOT_seerr_ana"), None),
        Some(&sent),
    )
    .await;
    let elsewhere = on_emby(
        &config,
        Method::POST,
        "/Users/AuthenticateByName",
        &seerr("someone", None),
        Some(&sent),
    )
    .await;

    assert_eq!(
        (answered.status, answered.body.as_str()),
        (StatusCode::OK, member)
    );
    assert_eq!(elsewhere.status, StatusCode::FORBIDDEN);
    assert_eq!(fake.seen().len(), 1);
    assert!(fake.sent("POST", "/Sessions/Logout").is_empty());
}

#[tokio::test]
async fn emby_ends_only_the_request_services_sessions_and_the_gate_answers_the_key_mint() {
    let (config, fake) = gate(
        "emby-devices-keys",
        vec![status(Method::DELETE, "/Devices", StatusCode::NO_CONTENT)],
    )
    .await;
    let authorisation = seerr("seerr", Some(JELLYFIN_TOKEN));

    let ended = on_emby(
        &config,
        Method::DELETE,
        &format!("/Devices?Id={}", encoded("BOT_seerr_ana")),
        &authorisation,
        None,
    )
    .await;
    let other = on_emby(
        &config,
        Method::DELETE,
        "/Devices?Id=someone",
        &authorisation,
        None,
    )
    .await;
    let minted = on_emby(
        &config,
        Method::POST,
        "/Auth/Keys?App=Seerr",
        &seerr(OWNER, None),
        None,
    )
    .await;
    let listed = on_emby(
        &config,
        Method::GET,
        "/Auth/Keys",
        &seerr(OWNER, None),
        None,
    )
    .await;

    assert_eq!(ended.status, StatusCode::NO_CONTENT);
    assert_eq!(other.status, StatusCode::FORBIDDEN);
    assert_eq!(minted.status, StatusCode::NO_CONTENT);
    let keys = listed.json();
    assert_eq!(at(&keys, "/Items/0/AppName"), json!("Seerr"));
    assert!(opens_nothing(&at(&keys, "/Items/0/AccessToken")), "{keys}");
    let seen = fake.seen();
    assert_eq!(
        seen.iter()
            .map(|one| (one.method.as_str(), one.path.as_str()))
            .collect::<Vec<_>>(),
        [("DELETE", "/Devices")]
    );
    assert_eq!(
        seen.first().and_then(|one| one.header("x-emby-token")),
        Some(KEY)
    );
}
