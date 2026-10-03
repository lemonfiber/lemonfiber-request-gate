use lemonfiber_sidecar::gate::{File, Outcome, Record};

use super::Files;
use crate::fake::{Config, TOKEN};

#[tokio::test]
async fn nothing_is_read_from_files_the_core_has_not_written() {
    let config = Config::new("unwritten");
    let files = Files::new(config.path());

    assert!(files.upstreams().await.is_none());
    assert!(!files.accepts("sonarr", Some(TOKEN)).await);
}

#[tokio::test]
async fn a_route_accepts_its_token_and_no_call_without_one() {
    let config = Config::new("accepts").with_routes("http://127.0.0.1:1");
    let files = Files::new(config.path());

    assert!(files.accepts("sonarr", Some(TOKEN)).await);
    assert!(!files.accepts("sonarr", Some("another")).await);
    assert!(!files.accepts("sonarr", None).await);
    assert!(files
        .upstreams()
        .await
        .is_some_and(|upstreams| upstreams.route("radarr").is_some()));
}

#[tokio::test]
async fn each_entry_is_added_after_the_last() {
    let config = Config::new("record");
    let files = Files::new(config.path());

    assert!(
        files
            .record("sonarr", "POST", "/sonarr/api/v3/tag", Outcome::Refused)
            .await
    );
    assert!(
        files
            .record(
                "radarr",
                "DELETE",
                "/radarr/api/v3/movie/7",
                Outcome::Removed
            )
            .await
    );

    let record = Record::read(&config.read(File::Record)).unwrap_or_default();
    let seen: Vec<_> = record
        .entries
        .iter()
        .map(|one| (one.seq, one.route.as_str(), one.outcome))
        .collect();
    assert_eq!(
        seen,
        [
            (1, "sonarr", Outcome::Refused),
            (2, "radarr", Outcome::Removed)
        ]
    );
}

#[tokio::test]
async fn a_record_that_cannot_be_read_is_started_again() {
    let config = Config::new("unreadable-record");
    config.write(File::Record, "not a record");
    let files = Files::new(config.path());

    assert!(
        files
            .record("sonarr", "GET", "/sonarr/x", Outcome::Refused)
            .await
    );
    assert_eq!(
        Record::read(&config.read(File::Record))
            .unwrap_or_default()
            .entries
            .len(),
        1
    );
}

#[tokio::test]
async fn a_record_that_cannot_be_written_says_so() {
    let config = Config::new("unwritable-record");
    let files = Files::new(config.path().join("missing"));

    assert!(
        !files
            .record("sonarr", "GET", "/sonarr/x", Outcome::Refused)
            .await
    );
}
