//! What the gate answers the request service on a curator's route, in the shapes Sonarr
//! and Radarr answer: built from what the curator answered, and nothing else.

use std::time::{SystemTime, UNIX_EPOCH};

use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::{
    CatalogueEntry, CuratedTitle, DownloadState, Downloading, RegisteredFolder,
};
use serde_json::{json, Map, Value};

/// The version of the API the series route presents: Sonarr's fourth, which asks no
/// language profile.
const SERIES_VERSION: &str = "4.0.0";

/// The version of the API the film route presents: Radarr's fifth.
const FILM_VERSION: &str = "5.0.0";

/// Seconds in a minute.
const MINUTE: i64 = 60;

/// Seconds in an hour.
const HOUR: i64 = 60 * MINUTE;

/// Seconds in a day.
const DAY: i64 = 24 * HOUR;

/// The system status of the route for `media`: the version of the API it presents, and no
/// path it answers under.
pub(super) fn status(media: Kind) -> Value {
    let version = match media {
        Kind::Tv => SERIES_VERSION,
        Kind::Movies => FILM_VERSION,
    };
    json!({ "version": version, "urlBase": "" })
}

/// `title`, a series or a film as `media` says, as Sonarr or Radarr answers one: its slug
/// the reference.
pub(super) fn title(media: Kind, title: &CuratedTitle) -> Value {
    let mut shown = Map::new();
    if let Some(id) = title.id {
        shown.insert("id".to_owned(), Value::from(id));
    }
    shown.insert("title".to_owned(), Value::from(title.title.clone()));
    shown.insert(
        "titleSlug".to_owned(),
        Value::from(title.reference.to_string()),
    );
    shown.insert(
        "year".to_owned(),
        Value::from(title.year.unwrap_or_default()),
    );
    shown.insert("monitored".to_owned(), Value::from(title.monitored));
    shown.insert("tags".to_owned(), json!([]));
    shown.insert("images".to_owned(), json!([]));
    match media {
        Kind::Tv => {
            shown.insert("tvdbId".to_owned(), Value::from(title.reference));
            let seasons: Vec<Value> = title
                .seasons
                .iter()
                .map(|season| {
                    json!({
                        "seasonNumber": season.number,
                        "monitored": season.monitored,
                        "statistics": {
                            "episodeFileCount": season.files,
                            "episodeCount": season.episodes,
                            "totalEpisodeCount": season.episodes,
                        },
                    })
                })
                .collect();
            let episodes: u32 = title.seasons.iter().map(|season| season.episodes).sum();
            shown.insert(
                "statistics".to_owned(),
                json!({
                    "seasonCount": seasons.len(),
                    "episodeFileCount": title.files,
                    "episodeCount": episodes,
                    "totalEpisodeCount": episodes,
                }),
            );
            shown.insert("seasons".to_owned(), Value::Array(seasons));
        }
        Kind::Movies => {
            shown.insert("tmdbId".to_owned(), Value::from(title.reference));
            shown.insert("hasFile".to_owned(), Value::from(title.files > 0));
        }
    }
    Value::Object(shown)
}

/// `entry`, a series the catalogue holds for a name, as Sonarr answers one.
pub(super) fn entry(entry: &CatalogueEntry) -> Value {
    let mut shown = Map::new();
    if let Some(id) = entry.held_as {
        shown.insert("id".to_owned(), Value::from(id));
    }
    shown.insert("title".to_owned(), Value::from(entry.title.clone()));
    shown.insert(
        "titleSlug".to_owned(),
        Value::from(entry.reference.to_string()),
    );
    shown.insert(
        "year".to_owned(),
        Value::from(entry.year.unwrap_or_default()),
    );
    shown.insert("tvdbId".to_owned(), Value::from(entry.reference));
    shown.insert("seasons".to_owned(), json!([]));
    shown.insert("images".to_owned(), json!([]));
    Value::Object(shown)
}

/// `folders`, as Sonarr and Radarr answer their root folders: with no free or total space
/// and no unmapped folder, each numbered by its own id where that is a number and by its
/// place otherwise.
pub(super) fn folders(folders: &[RegisteredFolder]) -> Value {
    folders
        .iter()
        .zip(1_u64..)
        .map(|(folder, place)| {
            json!({
                "id": folder.id.parse::<u64>().unwrap_or(place),
                "path": folder.path,
                "unmappedFolders": [],
            })
        })
        .collect()
}

/// What the curator filing `media` is downloading, as Sonarr and Radarr answer their
/// queue, asked at `now`, in seconds since the Unix epoch.
pub(super) fn queue(media: Kind, downloading: &[Downloading], now: i64) -> Value {
    let title_id = match media {
        Kind::Tv => "seriesId",
        Kind::Movies => "movieId",
    };
    let records: Vec<Value> = downloading
        .iter()
        .zip(1_u64..)
        .map(|(one, place)| {
            let mut record = Map::new();
            record.insert("id".to_owned(), Value::from(place));
            record.insert(title_id.to_owned(), Value::from(one.title_id));
            record.insert("title".to_owned(), Value::from(one.release.clone()));
            record.insert("size".to_owned(), Value::from(one.bytes));
            record.insert("sizeleft".to_owned(), Value::from(one.left));
            record.insert("status".to_owned(), Value::from(state(one.state)));
            if let Some(expected) = one.expected {
                record.insert("timeleft".to_owned(), Value::from(left(expected, now)));
                record.insert(
                    "estimatedCompletionTime".to_owned(),
                    Value::from(instant(expected)),
                );
            }
            if let Some(download) = &one.download_id {
                record.insert("downloadId".to_owned(), Value::from(download.clone()));
            }
            if let Some(episode) = &one.episode {
                record.insert(
                    "episode".to_owned(),
                    json!({
                        "seasonNumber": episode.season,
                        "episodeNumber": episode.number,
                        "title": episode.title,
                    }),
                );
            }
            Value::Object(record)
        })
        .collect();
    json!({
        "page": 1,
        "pageSize": records.len(),
        "totalRecords": records.len(),
        "records": records,
    })
}

/// `state`, as Sonarr and Radarr name where a download stands.
const fn state(state: DownloadState) -> &'static str {
    match state {
        DownloadState::Queued => "queued",
        DownloadState::Paused => "paused",
        DownloadState::Downloading => "downloading",
        DownloadState::Completed => "completed",
        DownloadState::Warning => "warning",
        DownloadState::Failed => "failed",
    }
}

/// Now, in seconds since the Unix epoch.
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_secs()).ok())
        .unwrap_or_default()
}

/// The time from `now` until `expected`, as Sonarr and Radarr write a span: `hh:mm:ss`, a
/// count of days and a full stop before it where there are any, and none once it has
/// passed.
fn left(expected: i64, now: i64) -> String {
    let left = expected.saturating_sub(now).max(0);
    let clock = format!(
        "{:02}:{:02}:{:02}",
        left % DAY / HOUR,
        left % HOUR / MINUTE,
        left % MINUTE
    );
    match left / DAY {
        0 => clock,
        days => format!("{days}.{clock}"),
    }
}

/// `seconds` since the Unix epoch as the UTC instant Sonarr and Radarr write.
fn instant(seconds: i64) -> String {
    let days = seconds.div_euclid(DAY);
    let time = seconds.rem_euclid(DAY);
    let (year, month, day) = civil(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / HOUR,
        time % HOUR / MINUTE,
        time % MINUTE
    )
}

/// The year, month and day `days` after 1 January 1970, in the proleptic Gregorian
/// calendar.
fn civil(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests;
