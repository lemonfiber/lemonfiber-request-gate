use crate::formats::gate::{File, Outcome, Record};

use super::{Files, Routes};
use crate::fake::{Config, TV_TOKEN};

#[tokio::test]
async fn nothing_is_read_from_files_the_core_has_not_written() {
    let config = Config::new("unwritten");
    let files = Files::new(config.path());

    assert!(matches!(files.upstreams().await, Routes::Unwritten));
    assert!(!files.accepts("tv", Some(TV_TOKEN)).await);
}

#[tokio::test]
async fn a_route_accepts_its_token_and_no_call_without_one() {
    let config = Config::new("accepts").with_routes("http://127.0.0.1:1");
    let files = Files::new(config.path());

    assert!(files.accepts("tv", Some(TV_TOKEN)).await);
    assert!(!files.accepts("tv", Some("another")).await);
    assert!(!files.accepts("tv", None).await);
    assert!(matches!(
        files.upstreams().await,
        Routes::Written(upstreams) if upstreams.route("movies").is_some()
    ));
}

#[tokio::test]
async fn each_entry_is_added_after_the_last() {
    let config = Config::new("record");
    let files = Files::new(config.path());

    assert!(
        files
            .record("tv", "POST", "/tv/api/v3/tag", Outcome::Refused)
            .await
    );
    assert!(
        files
            .record(
                "movies",
                "DELETE",
                "/movies/api/v3/movie/7",
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
        [(1, "tv", Outcome::Refused), (2, "movies", Outcome::Removed)]
    );
}

#[tokio::test]
async fn a_record_that_cannot_be_read_is_started_again() {
    let config = Config::new("unreadable-record");
    config.write(File::Record, "not a record");
    let files = Files::new(config.path());

    assert!(files.record("tv", "GET", "/tv/x", Outcome::Refused).await);
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

    assert!(!files.record("tv", "GET", "/tv/x", Outcome::Refused).await);
}

#[tokio::test]
async fn routes_that_are_there_and_cannot_be_read_are_unreadable() {
    let config = Config::new("unreadable-routes");
    let files = Files::new(config.path());
    let _ = std::fs::create_dir_all(config.path().join(File::Upstreams.name()));
    assert!(matches!(files.upstreams().await, Routes::Unreadable));

    let config = Config::new("unparsed-routes");
    config.write(File::Upstreams, "not the routes");
    assert!(matches!(
        Files::new(config.path()).upstreams().await,
        Routes::Unreadable
    ));
}
