//! The calls the gate answers on a curator's route, `/tv` or `/movies`, in the Sonarr and
//! Radarr API the request service speaks, and how it answers each.
//!
//! The gate answers the status, the tags, the episodes and a refresh of download tracking
//! itself. Every other call on the list is built as a `library.curate` operation of the
//! curator filing the route's media type, from the parameters and the body fields the list
//! names, checked against what the curator holds. The request service's token and
//! anything else it sent stay behind.

mod shown;
mod written;

use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lemonfiber_ports::media::Kind;
use serde_json::{json, Value};

use crate::asked::Asked;
use crate::curator::Curator;
use crate::shape::{object, parameters, required, Shape};
use crate::upstream::Stop;

/// The query parameter the request service sends its token in.
pub(crate) const TOKEN: &str = "apikey";

/// The path every call is made under, after the route.
const API: [&str; 2] = ["api", "v3"];

/// The parameter a lookup's term is sent in.
const TERM: &str = "term";

/// The command that refreshes download tracking, which the gate answers itself.
const REFRESH: &str = "RefreshMonitoredDownloads";

/// The two parameters a removal carries, and nothing else: it is asked only with its files
/// deleted, `true`, and nothing excluded from a later import, `false`.
const REMOVAL: [(&str, Shape); 2] = [
    ("deleteFiles", Shape::Flag),
    ("addImportExclusion", Shape::Flag),
];

/// One call on the list, on the route of the curator filing `media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Call {
    /// The kind of media the route's curator files.
    pub(crate) media: Kind,
    /// Which call it is.
    pub(crate) made: Made,
}

/// Which call on the list one is, each named as the request service makes it on Sonarr,
/// with `series` for a series and `movie` for a film.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Made {
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
    /// `GET /series`.
    Titles,
    /// `GET /series/{id}`.
    Title(i64),
    /// `GET /series/lookup`.
    Lookup,
    /// `POST /series`.
    Add,
    /// `PUT /series`.
    Monitor,
    /// `DELETE /series/{id}`.
    Remove(i64),
    /// `GET /episode`, on a series route.
    Episodes,
    /// `POST /command`.
    Command,
}

/// What came of a call the gate admitted.
#[derive(Debug)]
pub(crate) enum Curated {
    /// Answer with this.
    Answered(Response),
    /// Record the removal of the title the curator holds as this id, then ask it.
    Removal(i64),
}

/// The call `asked` makes on the route of the curator filing `media`, where it is one on
/// the list.
pub(crate) fn listed(media: Kind, asked: &Asked) -> Option<Call> {
    let segments = asked.segments();
    let rest = segments.strip_prefix(API.as_slice())?;
    let library = library(media);
    let made = match (asked.method.clone(), rest) {
        (Method::GET, ["system", "status"]) => Made::Status,
        (Method::GET, ["qualityprofile"]) => Made::QualityProfiles,
        (Method::GET, ["rootfolder"]) => Made::RootFolders,
        (Method::GET, ["tag"]) => Made::Tags,
        (Method::GET, ["queue"]) => Made::Queue,
        (Method::POST, ["command"]) => Made::Command,
        (Method::GET, ["episode"]) if media == Kind::Tv => Made::Episodes,
        (Method::GET, [titles]) if *titles == library => Made::Titles,
        (Method::GET, [titles, "lookup"]) if *titles == library => Made::Lookup,
        (Method::GET, [titles, id]) if *titles == library => Made::Title(number(id)?),
        (Method::POST, [titles]) if *titles == library => Made::Add,
        (Method::PUT, [titles]) if *titles == library => Made::Monitor,
        (Method::DELETE, [titles, id]) if *titles == library => Made::Remove(number(id)?),
        _ => return None,
    };
    Some(Call { media, made })
}

/// The path segment the request service names the titles of `media` by.
const fn library(media: Kind) -> &'static str {
    match media {
        Kind::Tv => "series",
        Kind::Movies => "movie",
    }
}

/// The parameter and field naming a title of `media` by its catalogue reference.
const fn reference(media: Kind) -> &'static str {
    match media {
        Kind::Tv => "tvdbId",
        Kind::Movies => "tmdbId",
    }
}

/// `digits` read as a curator's id or a catalogue reference: a whole number, written in
/// digits alone.
fn number(digits: &str) -> Option<i64> {
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// `value`, a whole number the shapes admitted, as a curator's id or a reference.
fn whole(value: &Value) -> Result<i64, Stop> {
    value
        .as_u64()
        .and_then(|id| i64::try_from(id).ok())
        .ok_or(Stop::Refused)
}

impl Call {
    /// What comes of this call, made by `asked` of `curator`.
    pub(crate) async fn answer(self, curator: &Curator, asked: &Asked) -> Result<Curated, Stop> {
        let query = &asked.query;
        let media = self.media;
        let answered = match self.made {
            Made::Status => Json(shown::status(media)).into_response(),
            Made::Tags => Json(json!([])).into_response(),
            Made::Episodes => {
                parameters(query, &[("seriesId", Shape::Integer)])?;
                Json(json!([])).into_response()
            }
            Made::QualityProfiles => Json(curator.quality_profiles().await?).into_response(),
            Made::RootFolders => {
                Json(shown::folders(&curator.root_folders().await?)).into_response()
            }
            Made::Queue => {
                parameters(query, &[("includeEpisode", Shape::Flag)])?;
                let downloading = curator.downloading().await?;
                Json(shown::queue(media, &downloading, shown::now())).into_response()
            }
            Made::Titles => {
                let by = reference(media);
                let only = parameters(query, &[(by, Shape::Integer)])?
                    .first()
                    .map(|(_, reference)| number(reference).ok_or(Stop::Refused))
                    .transpose()?;
                let titles: Vec<Value> = curator
                    .titles()
                    .await?
                    .iter()
                    .filter(|title| only.is_none_or(|reference| title.reference == reference))
                    .map(|title| shown::title(media, title))
                    .collect();
                Json(titles).into_response()
            }
            Made::Title(id) => match curator.held_title(id).await? {
                Some(title) => Json(shown::title(media, &title)).into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            },
            Made::Lookup => Json(looked_up(curator, asked).await?).into_response(),
            Made::Add => written::added(curator, asked).await?,
            Made::Monitor => written::monitored(curator, asked).await?,
            Made::Command => commanded(curator, asked).await?,
            Made::Remove(id) => {
                let flags = parameters(query, &REMOVAL)?;
                let asked_for = flags.iter().map(|(_, flag)| flag.as_str());
                if !asked_for.eq(["true", "false"]) {
                    return Err(Stop::Refused);
                }
                return Ok(Curated::Removal(id));
            }
        };
        Ok(Curated::Answered(answered))
    }
}

/// Remove the title `curator` holds as `id`, a removal the gate has recorded.
pub(crate) async fn removed(curator: &Curator, id: i64) -> Result<Response, Stop> {
    curator.remove(id).await?;
    Ok(Json(json!({})).into_response())
}

/// A lookup: of one title by its catalogue reference, `tvdb:<id>` or `tmdb:<id>`, or of
/// series by name.
async fn looked_up(curator: &Curator, asked: &Asked) -> Result<Vec<Value>, Stop> {
    let media = curator.media();
    let term = asked.query.one(TERM).ok().flatten().ok_or(Stop::Refused)?;
    let prefix = match media {
        Kind::Tv => "tvdb:",
        Kind::Movies => "tmdb:",
    };
    let by_reference = term
        .get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .and_then(|_| term.get(prefix.len()..));
    if let Some(reference) = by_reference {
        let reference = number(reference).ok_or(Stop::Refused)?;
        let found = curator.catalogued(reference).await?;
        return Ok(found
            .iter()
            .map(|title| shown::title(media, title))
            .collect());
    }
    if media != Kind::Tv || term.is_empty() {
        return Err(Stop::Refused);
    }
    Ok(curator
        .lookup(term)
        .await?
        .iter()
        .map(shown::entry)
        .collect())
}

/// `POST /command`: a refresh of download tracking, answered by the gate, or a search for
/// what was asked for, and no other command.
async fn commanded(curator: &Curator, asked: &Asked) -> Result<Response, Stop> {
    let sent = object(asked)?;
    let name = sent
        .get("name")
        .and_then(Value::as_str)
        .ok_or(Stop::Refused)?;
    let ids = match (name, curator.media()) {
        (REFRESH, _) => Vec::new(),
        ("MissingEpisodeSearch", Kind::Tv) => {
            vec![whole(&required(&sent, "seriesId", Shape::Integer)?)?]
        }
        ("MoviesSearch", Kind::Movies) => required(&sent, "movieIds", Shape::Integers)?
            .as_array()
            .into_iter()
            .flatten()
            .map(whole)
            .collect::<Result<_, _>>()?,
        _ => return Err(Stop::Refused),
    };
    for id in ids {
        curator.search(id).await?;
    }
    Ok((StatusCode::CREATED, Json(json!({ "name": name }))).into_response())
}

#[cfg(test)]
mod tests;
