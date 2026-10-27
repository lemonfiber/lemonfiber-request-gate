use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use lemonfiber_ports::service::{Failure, Gated, Gating, Target};
use lemonfiber_sidecar::gate::File;
use lemonfiber_sidecar::TokenHash;

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
        route(Target::MediaServer, "jellyfin", "jellyfin-token"),
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

    let later = vec![route(Target::MediaServer, "jellyfin", "later")];
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
        let mut one = route(Target::MediaServer, "jellyfin", "x");
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
