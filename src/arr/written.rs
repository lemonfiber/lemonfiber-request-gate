//! The writes on an \*arr route: each body built from the fields the list names, and
//! checked against what the upstream holds before it is sent.

use axum::http::Method;
use serde_json::{Map, Value};

use super::{api, path};
use crate::asked::Asked;
use crate::shape::{fields, nested, object, required, Shape};
use crate::upstream::{Built, Reach, Stop};

/// What adding one kind of title is built from, and checked by.
#[derive(Debug)]
pub(crate) struct Addition {
    /// The library a title is added to, and asked whether it holds one already.
    library: &'static str,
    /// The field naming the title at its metadata source, which is also the parameter
    /// the library is asked by.
    source: &'static str,
    /// The fields the body is built from.
    fields: &'static [(&'static str, Shape)],
    /// The lists of objects the body is built from, and the fields of each object.
    lists: &'static [(&'static str, &'static [(&'static str, Shape)])],
    /// The fields of `addOptions`.
    options: &'static [(&'static str, Shape)],
    /// The fields that name a quality profile, each of which must be one held.
    profiles: &'static [&'static str],
}

/// The field naming a quality profile.
const PROFILE: &str = "qualityProfileId";

/// The field naming a root folder.
const FOLDER: &str = "rootFolderPath";

/// The field holding a title's tags.
const TAGS: &str = "tags";

/// The field saying whether a title, a season or an episode is monitored.
pub(super) const MONITORED: &str = "monitored";

/// The field holding a title's options for what follows its addition.
const OPTIONS: &str = "addOptions";

/// The field holding a series' seasons.
const SEASONS: &str = "seasons";

/// The field numbering a season.
const SEASON: &str = "seasonNumber";

/// The field holding a title's identifier in the \*arr.
const ID: &str = "id";

/// The field naming a film's quality profile as older Radarr lines name it.
const LEGACY_PROFILE: &str = "profileId";

/// The field saying when a film counts as available.
const AVAILABILITY: &str = "minimumAvailability";

/// The fields of a film's `addOptions`.
const FILM_OPTIONS: &[(&str, Shape)] = &[("searchForMovie", Shape::Flag)];

/// A film request: `POST /movie`.
pub(crate) const FILM: Addition = Addition {
    library: path::FILMS,
    source: "tmdbId",
    fields: &[
        ("title", Shape::Text),
        ("tmdbId", Shape::Integer),
        ("year", Shape::Integer),
        ("titleSlug", Shape::Text),
        (PROFILE, Shape::Integer),
        (LEGACY_PROFILE, Shape::Integer),
        (AVAILABILITY, Shape::Text),
        (FOLDER, Shape::Text),
        (MONITORED, Shape::Flag),
        (TAGS, Shape::Integers),
    ],
    lists: &[],
    options: FILM_OPTIONS,
    profiles: &[PROFILE, LEGACY_PROFILE],
};

/// A series request: `POST /series`.
pub(crate) const SERIES: Addition = Addition {
    library: path::SERIES,
    source: "tvdbId",
    fields: &[
        ("tvdbId", Shape::Integer),
        ("title", Shape::Text),
        (PROFILE, Shape::Integer),
        ("languageProfileId", Shape::Integer),
        (TAGS, Shape::Integers),
        ("seasonFolder", Shape::Flag),
        (MONITORED, Shape::Flag),
        ("monitorNewItems", Shape::Text),
        (FOLDER, Shape::Text),
        ("seriesType", Shape::Text),
    ],
    lists: &[(
        SEASONS,
        &[(SEASON, Shape::Integer), (MONITORED, Shape::Flag)],
    )],
    options: &[
        ("ignoreEpisodesWithFiles", Shape::Flag),
        ("searchForMissingEpisodes", Shape::Flag),
    ],
    profiles: &[PROFILE],
};

/// A title request: the body built from `addition`'s fields, at a quality profile,
/// root folder and tags the upstream holds, for a title it does not hold yet.
pub(crate) async fn add(
    reach: &Reach<'_>,
    asked: &Asked,
    addition: &Addition,
) -> Result<Built, Stop> {
    let sent = object(asked)?;
    let mut body = fields(&sent, addition.fields)?;
    for &(list, named) in addition.lists {
        if let Some(items) = sent.get(list) {
            body.insert(list.to_owned(), Value::Array(objects(items, named)?));
        }
    }
    if let Some(options) = nested(&sent, OPTIONS)? {
        body.insert(
            OPTIONS.to_owned(),
            Value::Object(fields(options, addition.options)?),
        );
    }

    required(&body, PROFILE, Shape::Integer)?;
    let profiles = ids(reach, path::QUALITY_PROFILES).await?;
    addition
        .profiles
        .iter()
        .filter_map(|profile| body.get(*profile).and_then(Value::as_u64))
        .try_for_each(|id| held(&profiles, id))?;
    let folder = required(&body, FOLDER, Shape::Text)?;
    body.insert(
        FOLDER.to_owned(),
        Value::from(root_folder(reach, &folder).await?),
    );
    if let Some(tags) = body.get(TAGS) {
        tags_held(reach, tags).await?;
    }

    let source = required(&body, addition.source, Shape::Integer)?;
    let holding = reach
        .read(
            api(reach, Method::GET, addition.library)?
                .query(vec![(addition.source, source.to_string())]),
        )
        .await?;
    if !holding.as_array().ok_or(Stop::Unreachable)?.is_empty() {
        return Err(Stop::Refused);
    }
    Ok(api(reach, Method::POST, addition.library)?.body(Value::Object(body)))
}

/// A request for a film Radarr holds, unmonitored and without a file: the film as
/// Radarr holds it, with only its monitoring, quality profile, minimum availability and
/// options changed and tags added.
pub(crate) async fn film_monitored(reach: &Reach<'_>, asked: &Asked) -> Result<Built, Stop> {
    let sent = object(asked)?;
    let mut film = holding(reach, path::FILMS, &sent).await?;
    if film.get(MONITORED) != Some(&Value::Bool(false))
        || film.get("hasFile") != Some(&Value::Bool(false))
    {
        return Err(Stop::Refused);
    }
    let changes = fields(
        &sent,
        &[
            (MONITORED, Shape::Flag),
            (PROFILE, Shape::Integer),
            (AVAILABILITY, Shape::Text),
        ],
    )?;
    if let Some(profile) = changes.get(PROFILE).and_then(Value::as_u64) {
        held(&ids(reach, path::QUALITY_PROFILES).await?, profile)?;
    }
    film.extend(changes);
    if let Some(options) = nested(&sent, OPTIONS)? {
        film.insert(
            OPTIONS.to_owned(),
            Value::Object(fields(options, FILM_OPTIONS)?),
        );
    }
    tags_added(reach, &sent, &mut film).await?;
    Ok(api(reach, Method::PUT, path::FILMS)?.body(Value::Object(film)))
}

/// More seasons of a series Sonarr holds: the series as Sonarr holds it, with only the
/// series and the seasons asked for monitored, never unmonitored, and tags added.
pub(crate) async fn series_monitored(reach: &Reach<'_>, asked: &Asked) -> Result<Built, Stop> {
    let sent = object(asked)?;
    let mut series = holding(reach, path::SERIES, &sent).await?;
    let changes = fields(&sent, &[(MONITORED, Shape::Flag)])?;
    if changes.get(MONITORED) == Some(&Value::Bool(true)) {
        series.insert(MONITORED.to_owned(), Value::Bool(true));
    }
    let asked_for = sent
        .get(SEASONS)
        .map(monitored_seasons)
        .transpose()?
        .unwrap_or_default();
    series
        .get_mut(SEASONS)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object_mut)
        .filter(|season| {
            season
                .get(SEASON)
                .and_then(Value::as_u64)
                .is_some_and(|number| asked_for.contains(&number))
        })
        .for_each(|season| {
            season.insert(MONITORED.to_owned(), Value::Bool(true));
        });
    tags_added(reach, &sent, &mut series).await?;
    Ok(api(reach, Method::PUT, path::SERIES)?.body(Value::Object(series)))
}

/// The numbers of the seasons `seasons` asks to have monitored.
fn monitored_seasons(seasons: &Value) -> Result<Vec<u64>, Stop> {
    let seasons = objects(
        seasons,
        &[(SEASON, Shape::Integer), (MONITORED, Shape::Flag)],
    )?;
    Ok(seasons
        .iter()
        .filter(|season| season.get(MONITORED) == Some(&Value::Bool(true)))
        .filter_map(|season| season.get(SEASON).and_then(Value::as_u64))
        .collect())
}

/// `items`, a list of objects, each built from the fields `named`.
fn objects(items: &Value, named: &[(&'static str, Shape)]) -> Result<Vec<Value>, Stop> {
    items
        .as_array()
        .ok_or(Stop::Refused)?
        .iter()
        .map(|item| fields(item.as_object().ok_or(Stop::Refused)?, named).map(Value::Object))
        .collect()
}

/// The title in `library` that `sent` names by its id, as the upstream holds it.
async fn holding(
    reach: &Reach<'_>,
    library: &str,
    sent: &Map<String, Value>,
) -> Result<Map<String, Value>, Stop> {
    let id = required(sent, ID, Shape::Integer)?;
    match reach
        .read(api(reach, Method::GET, &format!("{library}/{id}"))?)
        .await?
    {
        Value::Object(title) => Ok(title),
        _ => Err(Stop::Unreachable),
    }
}

/// `title` with the tags `sent` names added to those it holds, each one the upstream
/// holds.
async fn tags_added(
    reach: &Reach<'_>,
    sent: &Map<String, Value>,
    title: &mut Map<String, Value>,
) -> Result<(), Stop> {
    let Some(tags) = fields(sent, &[(TAGS, Shape::Integers)])?.remove(TAGS) else {
        return Ok(());
    };
    tags_held(reach, &tags).await?;
    let mut all: Vec<u64> = title
        .get(TAGS)
        .and_then(Value::as_array)
        .map(|held| held.iter().filter_map(Value::as_u64).collect())
        .unwrap_or_default();
    for tag in tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
    {
        if !all.contains(&tag) {
            all.push(tag);
        }
    }
    title.insert(TAGS.to_owned(), Value::from(all));
    Ok(())
}

/// Whether every one of `tags` is a tag the upstream holds.
async fn tags_held(reach: &Reach<'_>, tags: &Value) -> Result<(), Stop> {
    let held_tags = ids(reach, path::TAGS).await?;
    for tag in tags
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
    {
        held(&held_tags, tag)?;
    }
    Ok(())
}

/// The root folder the upstream holds that `folder` names, spelled as the upstream
/// spells it; a trailing separator aside, the two must agree.
async fn root_folder(reach: &Reach<'_>, folder: &Value) -> Result<String, Stop> {
    let folder = folder.as_str().unwrap_or_default().trim_end_matches('/');
    let held = reach
        .read(api(reach, Method::GET, path::ROOT_FOLDERS)?)
        .await?;
    held.as_array()
        .ok_or(Stop::Unreachable)?
        .iter()
        .filter_map(|one| one.get("path").and_then(Value::as_str))
        .find(|path| path.trim_end_matches('/') == folder)
        .map(str::to_owned)
        .ok_or(Stop::Refused)
}

/// The ids of everything the upstream lists at `path`.
async fn ids(reach: &Reach<'_>, path: &str) -> Result<Vec<u64>, Stop> {
    let listed = reach.read(api(reach, Method::GET, path)?).await?;
    Ok(listed
        .as_array()
        .ok_or(Stop::Unreachable)?
        .iter()
        .filter_map(|one| one.get(ID).and_then(Value::as_u64))
        .collect())
}

/// Whether `id` is one of `held`.
fn held(held: &[u64], id: u64) -> Result<(), Stop> {
    if held.contains(&id) {
        Ok(())
    } else {
        Err(Stop::Refused)
    }
}
