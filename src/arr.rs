//! The calls the gate answers on a Sonarr or Radarr route, and how it builds each.
//!
//! Every call goes upstream under the route's own key, built from the method, the
//! path, the query parameters the list names and, for a write, the body fields it
//! names, checked against what the upstream holds. The request service's token and
//! anything else it sent stay behind.

mod written;

use axum::http::{HeaderName, HeaderValue, Method};
use lemonfiber_sidecar::gate::Kind;
use serde_json::{json, Value};

use crate::asked::Asked;
use crate::shape::{object, parameters, required, Shape};
use crate::upstream::{Built, Plan, Reach, Stop};

/// Where the API every call is made on lives, under the upstream's address.
const API: &str = "/api/v3";

/// The header an \*arr reads its key from.
const KEY: HeaderName = HeaderName::from_static("x-api-key");

/// The query parameter the request service sends its token in.
pub(crate) const TOKEN: &str = "apikey";

/// What a film lookup's term must start with: it looks a film up by its TMDB id.
const BY_TMDB: &str = "tmdb:";

/// The parameter a lookup's term is sent in.
const TERM: &str = "term";

/// The paths under the API the gate also reads for its own checks.
mod path {
    /// The quality profiles.
    pub(super) const QUALITY_PROFILES: &str = "/qualityProfile";
    /// The root folders.
    pub(super) const ROOT_FOLDERS: &str = "/rootfolder";
    /// The tags.
    pub(super) const TAGS: &str = "/tag";
    /// Radarr's films.
    pub(super) const FILMS: &str = "/movie";
    /// Sonarr's series.
    pub(super) const SERIES: &str = "/series";
}

/// One call on the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Call {
    /// `GET /system/status`.
    Status,
    /// `GET /qualityProfile`.
    QualityProfiles,
    /// `GET /rootfolder`.
    RootFolders,
    /// `GET /tag`.
    Tags,
    /// `GET /queue`.
    Queue,
    /// `GET /languageprofile`, on Sonarr.
    LanguageProfiles,
    /// `GET /movie`, on Radarr.
    Films,
    /// `GET /movie/{id}`, on Radarr.
    Film(u64),
    /// `GET /movie/lookup`, on Radarr.
    FilmLookup,
    /// `POST /movie`, on Radarr.
    AddFilm,
    /// `PUT /movie`, on Radarr.
    MonitorFilm,
    /// `DELETE /movie/{id}`, on Radarr.
    RemoveFilm(u64),
    /// `GET /series`, on Sonarr.
    AllSeries,
    /// `GET /series/{id}`, on Sonarr.
    OneSeries(u64),
    /// `GET /series/lookup`, on Sonarr.
    SeriesLookup,
    /// `GET /episode`, on Sonarr.
    Episodes,
    /// `POST /series`, on Sonarr.
    AddSeries,
    /// `PUT /series`, on Sonarr.
    MonitorSeries,
    /// `DELETE /series/{id}`, on Sonarr.
    RemoveSeries(u64),
    /// `PUT /episode/monitor`, on Sonarr.
    MonitorEpisodes,
    /// `POST /command`.
    Command,
}

/// The call `asked` makes on a route reaching `kind`, where it is one on the list.
pub(crate) fn listed(kind: Kind, asked: &Asked) -> Option<Call> {
    let segments = asked.segments();
    let api: Vec<&str> = API.split('/').skip(1).collect();
    let rest = segments.strip_prefix(api.as_slice())?;
    let film = kind == Kind::Radarr;
    let television = kind == Kind::Sonarr;
    let call = match (asked.method.clone(), rest) {
        (Method::GET, ["system", "status"]) => Call::Status,
        (Method::GET, ["qualityprofile"]) => Call::QualityProfiles,
        (Method::GET, ["rootfolder"]) => Call::RootFolders,
        (Method::GET, ["tag"]) => Call::Tags,
        (Method::GET, ["queue"]) => Call::Queue,
        (Method::POST, ["command"]) => Call::Command,
        (Method::GET, ["languageprofile"]) if television => Call::LanguageProfiles,
        (Method::GET, ["movie"]) if film => Call::Films,
        (Method::GET, ["movie", "lookup"]) if film => Call::FilmLookup,
        (Method::GET, ["movie", id]) if film => Call::Film(number(id)?),
        (Method::POST, ["movie"]) if film => Call::AddFilm,
        (Method::PUT, ["movie"]) if film => Call::MonitorFilm,
        (Method::DELETE, ["movie", id]) if film => Call::RemoveFilm(number(id)?),
        (Method::GET, ["series"]) if television => Call::AllSeries,
        (Method::GET, ["series", "lookup"]) if television => Call::SeriesLookup,
        (Method::GET, ["series", id]) if television => Call::OneSeries(number(id)?),
        (Method::GET, ["episode"]) if television => Call::Episodes,
        (Method::POST, ["series"]) if television => Call::AddSeries,
        (Method::PUT, ["series"]) if television => Call::MonitorSeries,
        (Method::DELETE, ["series", id]) if television => Call::RemoveSeries(number(id)?),
        (Method::PUT, ["episode", "monitor"]) if television => Call::MonitorEpisodes,
        _ => return None,
    };
    Some(call)
}

/// A path segment read as an \*arr's identifier: a whole number.
fn number(segment: &str) -> Option<u64> {
    segment.parse().ok()
}

/// The two parameters a removal carries, and nothing else.
const REMOVAL: [(&str, Shape); 2] = [
    ("deleteFiles", Shape::Flag),
    ("addImportExclusion", Shape::Flag),
];

impl Call {
    /// What the gate does with this call, made on `reach` by `asked`.
    pub(crate) async fn plan(self, reach: &Reach<'_>, asked: &Asked) -> Result<Plan, Stop> {
        let query = &asked.query;
        let plan = match self {
            Self::Status => Plan::Forward(api(reach, Method::GET, "/system/status")?),
            Self::QualityProfiles => {
                Plan::Forward(api(reach, Method::GET, path::QUALITY_PROFILES)?)
            }
            Self::RootFolders => Plan::Forward(api(reach, Method::GET, path::ROOT_FOLDERS)?),
            Self::Tags => Plan::Forward(api(reach, Method::GET, path::TAGS)?),
            Self::LanguageProfiles => Plan::Forward(api(reach, Method::GET, "/languageprofile")?),
            Self::Queue => Plan::Forward(
                api(reach, Method::GET, "/queue")?
                    .query(parameters(query, &[("includeEpisode", Shape::Flag)])?),
            ),
            Self::Films => Plan::Forward(
                api(reach, Method::GET, path::FILMS)?
                    .query(parameters(query, &[("tmdbId", Shape::Integer)])?),
            ),
            Self::Film(id) => {
                Plan::Forward(api(reach, Method::GET, &format!("{}/{id}", path::FILMS))?)
            }
            Self::FilmLookup => Plan::Forward(
                api(reach, Method::GET, &format!("{}/lookup", path::FILMS))?
                    .query(vec![(TERM, by_tmdb(asked)?)]),
            ),
            Self::AllSeries => Plan::Forward(
                api(reach, Method::GET, path::SERIES)?
                    .query(parameters(query, &[("tvdbId", Shape::Integer)])?),
            ),
            Self::OneSeries(id) => {
                Plan::Forward(api(reach, Method::GET, &format!("{}/{id}", path::SERIES))?)
            }
            Self::SeriesLookup => Plan::Forward(
                api(reach, Method::GET, &format!("{}/lookup", path::SERIES))?
                    .query(parameters(query, &[(TERM, Shape::Text)])?),
            ),
            Self::Episodes => Plan::Forward(
                api(reach, Method::GET, "/episode")?
                    .query(parameters(query, &[("seriesId", Shape::Integer)])?),
            ),
            Self::AddFilm => Plan::Forward(written::add(reach, asked, &written::FILM).await?),
            Self::AddSeries => Plan::Forward(written::add(reach, asked, &written::SERIES).await?),
            Self::MonitorFilm => Plan::Forward(written::film_monitored(reach, asked).await?),
            Self::MonitorSeries => Plan::Forward(written::series_monitored(reach, asked).await?),
            Self::MonitorEpisodes => Plan::Forward(episodes_monitored(reach, asked)?),
            Self::Command => Plan::Forward(command(reach, asked)?),
            Self::RemoveFilm(id) => Plan::Remove(
                api(reach, Method::DELETE, &format!("{}/{id}", path::FILMS))?
                    .query(parameters(query, &REMOVAL)?),
            ),
            Self::RemoveSeries(id) => Plan::Remove(
                api(reach, Method::DELETE, &format!("{}/{id}", path::SERIES))?
                    .query(parameters(query, &REMOVAL)?),
            ),
        };
        Ok(plan)
    }
}

/// `method` on `path` under the API, carrying the route's key.
fn api(reach: &Reach<'_>, method: Method, path: &str) -> Result<Built, Stop> {
    let key =
        HeaderValue::from_str(reach.upstream.credential.reveal()).map_err(|_| Stop::Unreachable)?;
    Ok(Built::new(method, format!("{API}{path}")).header(KEY, key))
}

/// The film lookup's term, which must name a TMDB id.
fn by_tmdb(asked: &Asked) -> Result<String, Stop> {
    asked
        .query
        .one(TERM)
        .ok()
        .flatten()
        .filter(|term| {
            term.get(..BY_TMDB.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(BY_TMDB))
                && term.get(BY_TMDB.len()..).is_some_and(|id| {
                    !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        .map(str::to_owned)
        .ok_or(Stop::Refused)
}

/// `PUT /episode/monitor`: the episodes named, monitored, and nothing else.
fn episodes_monitored(reach: &Reach<'_>, asked: &Asked) -> Result<Built, Stop> {
    let sent = object(asked)?;
    let episodes = required(&sent, "episodeIds", Shape::Integers)?;
    if sent.get(written::MONITORED) != Some(&Value::Bool(true)) {
        return Err(Stop::Refused);
    }
    Ok(api(reach, Method::PUT, "/episode/monitor")?
        .body(json!({ "episodeIds": episodes, written::MONITORED: true })))
}

/// `POST /command`: a search for what was asked for, or a refresh of download
/// tracking, and no other command.
fn command(reach: &Reach<'_>, asked: &Asked) -> Result<Built, Stop> {
    let sent = object(asked)?;
    let name = sent
        .get("name")
        .and_then(Value::as_str)
        .ok_or(Stop::Refused)?;
    let body = match (name, reach.upstream.kind) {
        ("RefreshMonitoredDownloads", _) => json!({ "name": name }),
        ("MoviesSearch", Kind::Radarr) => {
            json!({ "name": name, "movieIds": required(&sent, "movieIds", Shape::Integers)? })
        }
        ("MissingEpisodeSearch", Kind::Sonarr) => {
            json!({ "name": name, "seriesId": required(&sent, "seriesId", Shape::Integer)? })
        }
        _ => return Err(Stop::Refused),
    };
    Ok(api(reach, Method::POST, "/command")?.body(body))
}

#[cfg(test)]
mod tests;
