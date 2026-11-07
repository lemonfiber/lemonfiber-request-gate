//! The writes on a curator's route: each a `request` built from the fields the list names,
//! checked against what the curator holds before it is asked.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use lemonfiber_ports::media::Kind;
use lemonfiber_ports::service::TitleRequest;
use serde_json::{Map, Value};

use super::{reference, shown, whole};
use crate::asked::Asked;
use crate::curator::Curator;
use crate::shape::{fields, nested, object, required, Shape};
use crate::upstream::Stop;

/// The field naming a quality profile.
const PROFILE: &str = "qualityProfileId";

/// The field naming a root folder.
const FOLDER: &str = "rootFolderPath";

/// The field saying whether a season is monitored.
const MONITORED: &str = "monitored";

/// The field holding a series' seasons.
const SEASONS: &str = "seasons";

/// The field numbering a season.
const SEASON: &str = "seasonNumber";

/// The field holding a title's options for what follows its request.
const OPTIONS: &str = "addOptions";

/// The field naming how a series is numbered, and the one value that numbers it as anime.
const SERIES_TYPE: (&str, &str) = ("seriesType", "anime");

/// The field naming which seasons that air later are monitored, and the one value that
/// monitors them all.
const NEW_SEASONS: (&str, &str) = ("monitorNewItems", "all");

/// The option of `addOptions` that asks for a search once the request is taken.
const fn searching(media: Kind) -> &'static str {
    match media {
        Kind::Tv => "searchForMissingEpisodes",
        Kind::Movies => "searchForMovie",
    }
}

/// `POST /series`, `POST /movie`: a request for a title the curator does not hold, at a
/// quality profile and a root folder it holds.
pub(super) async fn added(curator: &Curator, asked: &Asked) -> Result<Response, Stop> {
    let media = curator.media();
    let sent = object(asked)?;
    let named = fields(
        &sent,
        &[(SERIES_TYPE.0, Shape::Text), (NEW_SEASONS.0, Shape::Text)],
    )?;
    let reference = whole(&required(&sent, reference(media), Shape::Integer)?)?;
    let profile = required(&sent, PROFILE, Shape::Integer)?;
    let folder = required(&sent, FOLDER, Shape::Text)?;
    let search = search(media, &sent)?;

    if !curator
        .quality_profiles()
        .await?
        .iter()
        .any(|held| Some(u64::from(held.id)) == profile.as_u64())
    {
        return Err(Stop::Refused);
    }
    let root_folder = root_folder(curator, folder.as_str().unwrap_or_default()).await?;
    if curator
        .catalogued(reference)
        .await?
        .is_some_and(|title| title.id.is_some())
    {
        return Err(Stop::Refused);
    }

    let tv = media == Kind::Tv;
    let request = TitleRequest {
        reference,
        root_folder,
        quality_profile: whole(&profile)?,
        seasons: if tv { seasons(&sent)? } else { Vec::new() },
        anime: tv && named.get(SERIES_TYPE.0) == Some(&Value::from(SERIES_TYPE.1)),
        later_seasons: tv && named.get(NEW_SEASONS.0) == Some(&Value::from(NEW_SEASONS.1)),
        search,
    };
    let title = curator.request(&request).await?;
    Ok(Json(shown::title(media, &title)).into_response())
}

/// `PUT /series`, `PUT /movie`: a request for a title the curator holds, the seasons the
/// body monitors of a series; a film must be held unmonitored and without its file.
pub(super) async fn monitored(curator: &Curator, asked: &Asked) -> Result<Response, Stop> {
    let media = curator.media();
    let sent = object(asked)?;
    let id = whole(&required(&sent, "id", Shape::Integer)?)?;
    let search = search(media, &sent)?;
    let Some(held) = curator.held_title(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let tv = media == Kind::Tv;
    if !tv && (held.monitored || held.files > 0) {
        return Err(Stop::Refused);
    }
    let request = TitleRequest {
        reference: held.reference,
        root_folder: String::new(),
        quality_profile: 0,
        seasons: if tv { seasons(&sent)? } else { Vec::new() },
        anime: false,
        later_seasons: false,
        search: !tv && search,
    };
    let title = curator.request(&request).await?;
    Ok(Json(shown::title(media, &title)).into_response())
}

/// Whether `sent` asks for a search once its request is taken.
fn search(media: Kind, sent: &Map<String, Value>) -> Result<bool, Stop> {
    let option = searching(media);
    let Some(options) = nested(sent, OPTIONS)? else {
        return Ok(false);
    };
    Ok(fields(options, &[(option, Shape::Flag)])?.get(option) == Some(&Value::Bool(true)))
}

/// The numbers of the seasons `sent` monitors.
fn seasons(sent: &Map<String, Value>) -> Result<Vec<u32>, Stop> {
    let Some(seasons) = sent.get(SEASONS) else {
        return Ok(Vec::new());
    };
    let mut monitored = Vec::new();
    for season in seasons.as_array().ok_or(Stop::Refused)? {
        let season = fields(
            season.as_object().ok_or(Stop::Refused)?,
            &[(SEASON, Shape::Integer), (MONITORED, Shape::Flag)],
        )?;
        if season.get(MONITORED) == Some(&Value::Bool(true)) {
            let number = required(&season, SEASON, Shape::Integer)?;
            monitored.push(
                number
                    .as_u64()
                    .and_then(|number| u32::try_from(number).ok())
                    .ok_or(Stop::Refused)?,
            );
        }
    }
    Ok(monitored)
}

/// The root folder the curator holds that `folder` names, spelled as the curator spells
/// it; a trailing separator aside, the two must agree.
async fn root_folder(curator: &Curator, folder: &str) -> Result<String, Stop> {
    let folder = folder.trim_end_matches('/');
    curator
        .root_folders()
        .await?
        .into_iter()
        .map(|held| held.path)
        .find(|path| path.trim_end_matches('/') == folder)
        .ok_or(Stop::Refused)
}
