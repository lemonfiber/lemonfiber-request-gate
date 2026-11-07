use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{DownloadState, Downloading};
use serde_json::json;

use super::{civil, instant, left, now, queue, state};

#[test]
fn a_span_is_written_as_a_clock_with_its_days_before_it() {
    assert_eq!(left(100, 100), "00:00:00");
    assert_eq!(left(100, 200), "00:00:00");
    assert_eq!(left(3_725, 0), "01:02:05");
    assert_eq!(left(86_399, 0), "23:59:59");
    assert_eq!(left(86_400, 0), "1.00:00:00");
    assert_eq!(left(2 * 86_400 + 61, 0), "2.00:01:01");
}

#[test]
fn an_instant_is_written_in_utc_to_the_second() {
    assert_eq!(instant(0), "1970-01-01T00:00:00Z");
    assert_eq!(instant(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(instant(1_793_793_599), "2026-11-04T11:59:59Z");
    assert_eq!(instant(4_102_444_799), "2099-12-31T23:59:59Z");
    assert_eq!(instant(-1), "1969-12-31T23:59:59Z");
}

#[test]
fn every_month_starts_where_the_last_ended() {
    assert_eq!(civil(0), (1970, 1, 1));
    assert_eq!(civil(31), (1970, 2, 1));
    assert_eq!(civil(59), (1970, 3, 1));
    assert_eq!(civil(334), (1970, 12, 1));
    assert_eq!(civil(365), (1971, 1, 1));
    assert_eq!(civil(11_016), (2000, 2, 29));
    assert_eq!(civil(11_017), (2000, 3, 1));
    assert_eq!(civil(-719_468), (0, 3, 1));
}

#[test]
fn each_state_is_named_as_sonarr_names_it() {
    for (one, named) in [
        (DownloadState::Queued, "queued"),
        (DownloadState::Paused, "paused"),
        (DownloadState::Downloading, "downloading"),
        (DownloadState::Completed, "completed"),
        (DownloadState::Warning, "warning"),
        (DownloadState::Failed, "failed"),
    ] {
        assert_eq!(state(one), named);
    }
}

#[test]
fn a_film_download_names_its_film_and_the_time_left_from_now() {
    let downloading = [Downloading {
        title_id: 7,
        release: "The.Matrix".to_owned(),
        bytes: 5,
        left: 1,
        expected: Some(90_061),
        state: DownloadState::Warning,
        download_id: None,
        episode: None,
    }];

    let answered = queue(Kind::Movies, &downloading, 0);

    assert_eq!(
        answered,
        json!({
            "page": 1,
            "pageSize": 1,
            "totalRecords": 1,
            "records": [{
                "id": 1,
                "movieId": 7,
                "title": "The.Matrix",
                "size": 5,
                "sizeleft": 1,
                "status": "warning",
                "timeleft": "1.01:01:01",
                "estimatedCompletionTime": "1970-01-02T01:01:01Z",
            }],
        })
    );
}

#[test]
fn now_is_after_the_gate_was_written() {
    assert!(now() > 1_790_000_000);
}
