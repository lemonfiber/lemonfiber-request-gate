use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use crate::formats::gate::{File, Kept, Kind, Outcome, Record, Upstreams};
use crate::formats::TokenHash;
use lemonfiber_ports::service::{
    Failure, GateEntry, GateOutcome, GateRecord, Gated, Gating, Target,
};

use super::Holding;
use crate::files::{Files, Routes};

struct Config(PathBuf);

impl Config {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("gate-holding-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::create_dir_all(&path);
        Self(path)
    }

    fn holding(&self) -> Holding {
        Holding::in_directory(&self.0)
    }

    fn read(&self, file: File) -> String {
        std::fs::read_to_string(self.0.join(file.name())).unwrap_or_default()
    }

    fn mode(&self, file: File) -> u32 {
        std::fs::metadata(self.0.join(file.name()))
            .map(|meta| meta.permissions().mode() & 0o777)
            .unwrap_or_default()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn credential() -> String {
    ('k'..='z').collect()
}

fn route(target: Target, path: &str, token: &str) -> Gated {
    Gated {
        target,
        path: path.to_owned(),
        address: format!("http://{path}:8989"),
        majors: vec![3],
        credential: credential(),
        tokens: vec![TokenHash::of(token).as_str().to_owned()],
    }
}

fn pin() -> String {
    "ab".repeat(32)
}

fn media(path: &str, token: &str) -> Gated {
    Gated {
        address: format!("https://{path}:8920"),
        ..route(Target::MediaServer { pins: vec![pin()] }, path, token)
    }
}

fn routes() -> Vec<Gated> {
    vec![
        route(
            Target::Curator {
                media_type: "tv".to_owned(),
            },
            "tv",
            "tv-token",
        ),
        route(
            Target::Curator {
                media_type: "movies".to_owned(),
            },
            "movies",
            "movies-token",
        ),
        media("jellyfin", "jellyfin-token"),
    ]
}

#[tokio::test]
async fn the_routes_admitted_are_the_ones_the_gate_serves_and_their_tokens_alone() {
    let config = Config::new("admitted");
    assert!(config.holding().admit(&routes()).await.is_ok());

    let files = Files::new(config.0.clone());
    assert!(matches!(files.upstreams().await, Routes::Written(read) if read.upstreams.len() == 3));
    assert!(files.accepts("tv", Some("tv-token")).await);
    assert!(files.accepts("jellyfin", Some("jellyfin-token")).await);
    assert!(!files.accepts("tv", Some("movies-token")).await);
    assert!(!files.accepts("tv", Some("another")).await);
}

#[tokio::test]
async fn what_it_admits_is_answered_with_each_credential_by_its_sha256_alone() {
    let config = Config::new("answered");
    let holding = config.holding();
    assert!(holding.admit(&routes()).await.is_ok());

    let admitted = holding.admitted().await.unwrap_or_default();

    assert_eq!(admitted.len(), 3);
    for (one, applied) in admitted.iter().zip(routes()) {
        assert_eq!(one.target, applied.target);
        assert_eq!(one.path, applied.path);
        assert_eq!(one.address, applied.address);
        assert_eq!(one.majors, applied.majors);
        assert_eq!(one.tokens, applied.tokens);
        assert_eq!(one.credential, TokenHash::of(&credential()).as_str());
        assert_ne!(one.credential, credential());
    }
}

#[tokio::test]
async fn nothing_admitted_answers_no_route() {
    let config = Config::new("nothing");

    assert_eq!(config.holding().admitted().await.ok(), Some(Vec::new()));
}

#[tokio::test]
async fn routes_admitted_again_leave_none_of_the_earlier_ones() {
    let config = Config::new("again");
    let holding = config.holding();
    assert!(holding.admit(&routes()).await.is_ok());

    let later = vec![media("jellyfin", "later")];
    assert!(holding.admit(&later).await.is_ok());

    let files = Files::new(config.0.clone());
    assert!(!files.accepts("tv", Some("tv-token")).await);
    assert!(!files.accepts("jellyfin", Some("jellyfin-token")).await);
    assert!(files.accepts("jellyfin", Some("later")).await);
}

#[tokio::test]
async fn what_it_holds_is_readable_by_the_gate_alone() {
    let config = Config::new("modes");
    let left = config.0.join(format!("{}.writing", File::Upstreams.name()));
    let _ = std::fs::write(&left, "stale");
    let _ = std::fs::set_permissions(&left, std::fs::Permissions::from_mode(0o666));

    assert!(config.holding().admit(&routes()).await.is_ok());

    assert_eq!(config.mode(File::Upstreams), 0o600);
    assert_eq!(config.mode(File::Tokens), 0o600);
    assert_ne!(config.read(File::Upstreams), "stale");
}

#[tokio::test]
async fn a_route_with_a_token_that_is_no_sha256_or_a_curator_of_nothing_it_reaches_is_refused() {
    let config = Config::new("refused");
    let holding = config.holding();
    assert!(holding.admit(&routes()).await.is_ok());
    let (upstreams, tokens) = (config.read(File::Upstreams), config.read(File::Tokens));

    let hash = TokenHash::of("tv-token").as_str().to_owned();
    let mut refused = Vec::new();
    for token in [
        "tv-token".to_owned(),
        hash.to_ascii_uppercase(),
        hash[1..].to_owned(),
    ] {
        let mut one = media("jellyfin", "x");
        one.tokens = vec![token];
        refused.push(one);
    }
    refused.push(route(
        Target::Curator {
            media_type: "books".to_owned(),
        },
        "books",
        "books-token",
    ));
    refused.push(Gated {
        address: "http://jellyfin:8096".to_owned(),
        ..media("jellyfin", "x")
    });
    for pin in ["AB".repeat(32), "ab".repeat(31), "not a pin".to_owned()] {
        refused.push(Gated {
            target: Target::MediaServer { pins: vec![pin] },
            ..media("jellyfin", "x")
        });
    }

    for one in refused {
        assert!(matches!(
            holding.admit(&[one]).await,
            Err(Failure::Refused { .. })
        ));
    }
    assert_eq!(config.read(File::Upstreams), upstreams);
    assert_eq!(config.read(File::Tokens), tokens);
}

#[tokio::test]
async fn what_cannot_be_written_is_refused() {
    let config = Config::new("unwritable");
    let holding = Holding::in_directory(&config.0.join("missing"));

    assert!(matches!(
        holding.admit(&routes()).await,
        Err(Failure::Refused { .. })
    ));
}

#[tokio::test]
async fn a_copy_left_behind_that_cannot_be_cleared_is_refused() {
    let config = Config::new("uncleared");
    let _ = std::fs::create_dir(config.0.join(format!("{}.writing", File::Tokens.name())));

    assert!(matches!(
        config.holding().admit(&routes()).await,
        Err(Failure::Refused { .. })
    ));
    assert_eq!(config.read(File::Upstreams), "");
}

fn hash(token: &str) -> String {
    TokenHash::of(token).as_str().to_owned()
}

#[tokio::test]
async fn tokens_admitted_on_one_route_replace_its_tokens_and_leave_every_credential_and_other_route(
) {
    let config = Config::new("tokens");
    let holding = config.holding();
    assert!(holding.admit(&routes()).await.is_ok());
    let upstreams = config.read(File::Upstreams);

    let admitted = holding
        .admit_tokens("jellyfin", &[hash("rotated"), hash("kept")])
        .await;

    assert!(admitted.is_ok(), "{admitted:?}");
    let files = Files::new(config.0.clone());
    assert!(files.accepts("jellyfin", Some("rotated")).await);
    assert!(files.accepts("jellyfin", Some("kept")).await);
    assert!(!files.accepts("jellyfin", Some("jellyfin-token")).await);
    assert!(files.accepts("tv", Some("tv-token")).await);
    assert!(files.accepts("movies", Some("movies-token")).await);
    assert_eq!(config.read(File::Upstreams), upstreams);
    assert_eq!(config.mode(File::Tokens), 0o600);
}

#[tokio::test]
async fn tokens_on_a_route_with_none_yet_are_added_beside_the_rest() {
    let config = Config::new("tokens-first");
    let holding = config.holding();
    assert!(holding.admit(&routes()).await.is_ok());
    let _ = std::fs::remove_file(config.0.join(File::Tokens.name()));

    assert!(holding.admit_tokens("tv", &[hash("first")]).await.is_ok());

    let files = Files::new(config.0.clone());
    assert!(files.accepts("tv", Some("first")).await);
    assert!(!files.accepts("movies", Some("movies-token")).await);
}

#[tokio::test]
async fn tokens_are_refused_on_a_route_it_does_not_admit_or_unlike_a_sha256_and_nothing_changes() {
    let config = Config::new("tokens-refused");
    let holding = config.holding();
    let unadmitted = holding.admit_tokens("tv", &[hash("early")]).await;
    assert!(
        matches!(&unadmitted, Err(Failure::Refused { detail, .. }) if detail.contains("\"tv\"")),
        "{unadmitted:?}"
    );
    assert!(holding.admit(&routes()).await.is_ok());
    let tokens = config.read(File::Tokens);

    for (route, admitted) in [
        ("sonarr", vec![hash("one")]),
        ("tv", vec!["not-a-sha256".to_owned()]),
        ("tv", vec![hash("one").to_uppercase()]),
    ] {
        let answer = holding.admit_tokens(route, &admitted).await;
        assert!(
            matches!(answer, Err(Failure::Refused { .. })),
            "{route}: {answer:?}"
        );
    }
    assert_eq!(config.read(File::Tokens), tokens);
}

#[tokio::test]
async fn tokens_are_refused_where_the_routes_or_the_tokens_cannot_be_read() {
    for (name, unreadable) in [("routes", File::Upstreams), ("held", File::Tokens)] {
        let config = Config::new(&format!("tokens-unreadable-{name}"));
        let holding = config.holding();
        assert!(holding.admit(&routes()).await.is_ok());
        let _ = std::fs::write(config.0.join(unreadable.name()), "not what it holds");

        let answer = holding.admit_tokens("tv", &[hash("one")]).await;

        assert!(
            matches!(&answer, Err(Failure::Refused { detail, .. }) if detail.contains("could not be read")),
            "{name}: {answer:?}"
        );
    }
    let config = Config::new("tokens-a-directory");
    let _ = std::fs::create_dir(config.0.join(File::Upstreams.name()));
    let answer = config.holding().admit_tokens("tv", &[hash("one")]).await;
    assert!(
        matches!(&answer, Err(Failure::Refused { detail, .. }) if detail.contains("could not be read")),
        "{answer:?}"
    );
}

fn recorded(entries: usize) -> Record {
    (0..entries).fold(Record::default(), |record, at| {
        let outcome = if at % 2 == 0 {
            Outcome::Refused
        } else {
            Outcome::Removed
        };
        record.with(
            u64::try_from(at).unwrap_or_default(),
            "tv",
            "DELETE",
            "/api/v3/series/1?deleteFiles=true",
            outcome,
            Kept::standard(),
        )
    })
}

#[tokio::test]
async fn the_record_is_answered_after_the_entry_asked_for_and_never_with_a_query() {
    let config = Config::new("record");
    let _ = std::fs::write(config.0.join(File::Record.name()), recorded(3).written());

    let answer = config.holding().record(1).await;

    assert_eq!(
        answer.ok(),
        Some(GateRecord {
            entries: vec![
                GateEntry {
                    seq: 2,
                    at: 1,
                    route: "tv".to_owned(),
                    method: "DELETE".to_owned(),
                    path: "/api/v3/series/1".to_owned(),
                    outcome: GateOutcome::Removed,
                },
                GateEntry {
                    seq: 3,
                    at: 2,
                    route: "tv".to_owned(),
                    method: "DELETE".to_owned(),
                    path: "/api/v3/series/1".to_owned(),
                    outcome: GateOutcome::Refused,
                },
            ],
            lost: 0,
        })
    );
}

#[tokio::test]
async fn entries_the_record_no_longer_holds_are_counted_as_lost() {
    let config = Config::new("record-lost");
    let kept = Kept::standard().entries();
    let _ = std::fs::write(
        config.0.join(File::Record.name()),
        recorded(kept + 5).written(),
    );

    let answer = config.holding().record(2).await.unwrap_or_default();

    assert_eq!(answer.lost, 3);
    assert_eq!(answer.entries.len(), kept);
    assert_eq!(answer.entries.first().map(|one| one.seq), Some(6));
}

#[tokio::test]
async fn no_record_yet_is_an_empty_one_and_an_unreadable_record_is_refused() {
    let config = Config::new("record-none");
    assert_eq!(
        config.holding().record(0).await.ok(),
        Some(GateRecord::default())
    );

    let _ = std::fs::write(config.0.join(File::Record.name()), "not a record");
    let answer = config.holding().record(0).await;
    assert!(
        matches!(&answer, Err(Failure::Refused { detail, .. }) if detail.contains("record could not be read")),
        "{answer:?}"
    );
}

fn plex(link: lemonfiber_ports::service::PlexLink) -> Gated {
    Gated {
        address: "https://plex:32400".to_owned(),
        majors: Vec::new(),
        ..route(
            Target::Plex {
                pins: vec![pin()],
                link,
            },
            "plex",
            "plex-token",
        )
    }
}

#[tokio::test]
async fn a_plex_route_is_held_with_how_the_request_service_is_linked_and_answered_back_as_applied()
{
    use lemonfiber_ports::service::{Admitted, OutsideAccount, OutsideServer, PlexLink};
    for link in [
        PlexLink::Managed,
        PlexLink::Owner {
            account: "4242".to_owned(),
        },
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
                id: "m".to_owned(),
            },
        },
    ] {
        let config = Config::new("plex-held");
        let applied = plex(link.clone());

        assert!(config
            .holding()
            .admit(std::slice::from_ref(&applied))
            .await
            .is_ok());

        let held = crate::formats::gate::Upstreams::read(&config.read(File::Upstreams));
        let held = held.ok().and_then(|held| held.plex().cloned());
        assert_eq!(
            held.as_ref().map(|one| one.kind),
            Some(crate::formats::gate::Kind::Plex)
        );
        assert_eq!(held.and_then(|one| one.link), Some(link));
        assert_eq!(
            config.holding().admitted().await.ok(),
            Some(vec![Admitted::of(&applied)])
        );
    }
}

#[tokio::test]
async fn a_plex_route_reached_without_tls_is_refused_and_nothing_is_written() {
    let config = Config::new("plex-plain");
    let plain = Gated {
        address: "http://plex:32400".to_owned(),
        ..plex(lemonfiber_ports::service::PlexLink::Managed)
    };

    assert!(matches!(
        config.holding().admit(&[plain]).await,
        Err(Failure::Refused { detail, .. }) if detail.contains("TLS")
    ));
    assert!(config.read(File::Upstreams).is_empty());
}

#[tokio::test]
async fn an_emby_route_is_held_as_emby_reached_over_tls_alone() {
    let config = Config::new("emby");
    let holding = config.holding();
    let emby = Gated {
        target: Target::Emby { pins: vec![pin()] },
        ..media("emby", "emby-token")
    };

    assert!(holding.admit(std::slice::from_ref(&emby)).await.is_ok());
    let admitted = holding.admitted().await.unwrap_or_default();
    let plain = Gated {
        address: "http://emby:8096".to_owned(),
        ..emby.clone()
    };

    assert_eq!(
        admitted
            .iter()
            .map(|one| one.target.clone())
            .collect::<Vec<_>>(),
        [emby.target]
    );
    assert!(matches!(
        Upstreams::read(&config.read(File::Upstreams)),
        Ok(read) if read.route("emby").map(|one| one.kind) == Some(Kind::Emby)
    ));
    assert!(matches!(
        holding.admit(&[plain]).await,
        Err(Failure::Refused { .. })
    ));
}
