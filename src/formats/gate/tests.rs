use super::{
    Accepted, Credential, File, Kept, Kind, Outcome, Record, Tokens, Upstream, Upstreams, FORMAT,
    PORT,
};
use crate::formats::{Pins, TokenHash};

fn curator() -> Upstream {
    Upstream {
        route: "tv".to_owned(),
        kind: Kind::Tv,
        address: "http://sonarr-adapter:5058".to_owned(),
        credential: Credential::new("the-tv-key"),
        majors: Vec::new(),
        pins: Pins::default(),
        link: None,
    }
}

fn jellyfin() -> Upstream {
    Upstream {
        route: "jellyfin".to_owned(),
        kind: Kind::Jellyfin,
        address: "https://jellyfin:8920".to_owned(),
        credential: Credential::new("the-gates-key"),
        majors: vec![10, 12],
        pins: Pins::of(&["ab".repeat(32)]).unwrap_or_default(),
        link: None,
    }
}

#[test]
fn the_files_have_their_names_and_the_gate_its_port() {
    assert_eq!(File::Upstreams.name(), "upstreams.json");
    assert_eq!(File::Tokens.name(), "tokens.json");
    assert_eq!(File::Record.name(), "record.json");
    assert_eq!(PORT, 5057);
}

#[test]
fn upstreams_read_back_and_find_a_route_whatever_its_case() {
    let upstreams = Upstreams::of(vec![curator()]);
    let read = Upstreams::read(&upstreams.written());

    assert_eq!(read.as_ref().ok(), Some(&upstreams));
    assert_eq!(upstreams.route("TV").map(|one| one.kind), Some(Kind::Tv));
    assert!(upstreams.route("movies").is_none());
    assert_eq!(
        upstreams.route("tv").map(|one| one.credential.reveal()),
        Some("the-tv-key")
    );
}

#[test]
fn a_route_carries_its_majors_and_a_curators_route_none() {
    let upstreams = Upstreams::of(vec![curator(), jellyfin()]);
    let written = upstreams.written();

    assert_eq!(Upstreams::read(&written).ok(), Some(upstreams.clone()));
    assert_eq!(written.matches("\"majors\"").count(), 1, "{written}");
    assert_eq!(
        upstreams.route("jellyfin").map(|one| one.majors.clone()),
        Some(vec![10, 12])
    );
}

#[test]
fn the_media_servers_route_carries_its_pins_a_curators_route_none_and_an_older_file_reads() {
    let upstreams = Upstreams::of(vec![curator(), jellyfin()]);
    let written = upstreams.written();

    assert_eq!(written.matches("\"pins\"").count(), 1, "{written}");
    assert_eq!(
        upstreams
            .route("jellyfin")
            .map(|one| one.pins.listed().to_vec()),
        Some(vec!["ab".repeat(32)])
    );
    let older = written.replace(
        &format!(
            ",\n      \"pins\": [\n        \"{}\"\n      ]",
            "ab".repeat(32)
        ),
        "",
    );
    assert!(!older.contains("pins"), "{older}");
    assert_eq!(
        Upstreams::read(&older)
            .ok()
            .and_then(|read| read.route("jellyfin").map(|one| one.pins.is_empty())),
        Some(true)
    );
    let unpinned = written.replace(&"ab".repeat(32), "not-a-pin");
    assert!(Upstreams::read(&unpinned).is_err());
}

#[test]
fn a_credential_is_withheld_from_debug() {
    let shown = format!("{:?}", curator());

    assert!(!shown.contains("the-tv-key"), "{shown}");
    assert!(shown.contains("withheld"), "{shown}");
}

#[test]
fn a_route_accepts_only_its_own_tokens_and_both_while_one_replaces_the_other() {
    let tokens = Tokens::of(vec![
        Accepted {
            route: "tv".to_owned(),
            tokens: vec![TokenHash::of("old"), TokenHash::of("new")],
        },
        Accepted {
            route: "movies".to_owned(),
            tokens: vec![TokenHash::of("films")],
        },
    ]);

    assert!(tokens.accepts("tv", "old"));
    assert!(tokens.accepts("Tv", "new"));
    assert!(!tokens.accepts("tv", "films"));
    assert!(!tokens.accepts("jellyfin", "old"));
    assert_eq!(Tokens::read(&tokens.written()).ok(), Some(tokens.clone()));
    assert!(!tokens.written().contains("\"old\""));
}

#[test]
fn the_record_numbers_its_entries_and_keeps_no_query_string() {
    let record = Record::default()
        .with(
            10,
            "movies",
            "DELETE",
            "/movies/api/v3/movie/7?deleteFiles=true",
            Outcome::Removed,
            Kept::standard(),
        )
        .with(
            11,
            "tv",
            "POST",
            "/tv/api/v3/tag",
            Outcome::Refused,
            Kept::standard(),
        );

    assert_eq!(
        record.entries.iter().map(|one| one.seq).collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        record.entries.first().map(|one| one.path.as_str()),
        Some("/movies/api/v3/movie/7")
    );
    assert_eq!(Record::read(&record.written()).ok(), Some(record));
}

#[test]
fn the_record_keeps_only_what_it_may_and_says_what_fell_off() {
    let mut record = Record::default();
    for at in 0..1005 {
        record = record.with(
            at,
            "tv",
            "POST",
            "/tv/api/v3/tag",
            Outcome::Refused,
            Kept::standard(),
        );
    }

    assert_eq!(record.entries.len(), Kept::standard().entries());
    assert_eq!(record.entries.first().map(|one| one.seq), Some(6));
    let (unread, lost) = record.since(2);
    assert_eq!(unread.len(), 1000);
    assert_eq!(lost, 3);
    let (unread, lost) = record.since(1004);
    assert_eq!((unread.len(), lost), (1, 0));
    assert_eq!(record.since(1005), (Vec::new(), 0));
}

#[test]
fn a_file_in_another_format_or_shape_is_refused() {
    for (text, read) in [
        (
            format!(r#"{{"format":{},"upstreams":[]}}"#, FORMAT + 1),
            Upstreams::read as fn(&str) -> Result<Upstreams, crate::formats::Unreadable>,
        ),
        ("not json".to_owned(), Upstreams::read),
    ] {
        assert!(read(&text).is_err(), "{text}");
    }
    assert!(Tokens::read(&format!(r#"{{"format":{},"routes":[]}}"#, FORMAT + 1)).is_err());
    assert!(Tokens::read("[]").is_err());
    assert!(Record::read(&format!(r#"{{"format":{},"entries":[]}}"#, FORMAT + 1)).is_err());
    assert!(Record::read("[]").is_err());
}

#[test]
fn a_curators_route_names_the_media_it_files_and_the_media_servers_none() {
    use lemonfiber_ports::media;

    assert_eq!(Kind::Tv.media(), Some(media::Kind::Tv));
    assert_eq!(Kind::Movies.media(), Some(media::Kind::Movies));
    for server in [Kind::Jellyfin, Kind::Emby, Kind::Plex] {
        assert_eq!(server.media(), None, "{server:?}");
    }
}

#[test]
fn a_route_written_for_an_arr_itself_is_not_read() {
    let written = Upstreams::of(vec![curator()]).written();
    let older = written.replace("\"tv\"", "\"sonarr\"");

    assert!(older.contains("\"sonarr\""), "{older}");
    assert!(Upstreams::read(&older).is_err());
}
