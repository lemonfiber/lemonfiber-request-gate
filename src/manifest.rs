//! The manifest's asks, held to the media the gate routes.

use lemonfiber_ports::media::{Kind, MEDIA};
use serde::Deserialize;

/// The manifest the image is published with.
const MANIFEST: &str = include_str!("../plugin.toml");

/// The part of a manifest its asks are.
#[derive(Deserialize)]
struct Manifest {
    /// Each `[[ask]]`.
    #[serde(default, rename = "ask")]
    asks: Vec<Ask>,
}

/// One `[[ask]]`.
#[derive(Deserialize)]
struct Ask {
    /// The capability asked for.
    capability: String,
    /// The media it asks a filler of.
    media_types: Option<Vec<String>>,
}

/// What is wrong with the asks of `manifest`: each ask that names no media, names one
/// outside the stack's vocabulary, or names other media than the gate routes.
fn faults(manifest: &str) -> Vec<String> {
    let Ok(read) = toml::from_str::<Manifest>(manifest) else {
        return vec!["the manifest is not TOML".to_owned()];
    };
    let routed: Vec<&str> = Kind::ALL.into_iter().map(Kind::media_type).collect();
    read.asks
        .iter()
        .filter_map(|ask| {
            let Some(media) = &ask.media_types else {
                return Some(format!("{} names no media", ask.capability));
            };
            if let Some(outside) = media.iter().find(|one| !MEDIA.contains(&one.as_str())) {
                return Some(format!(
                    "{} names {outside}, outside the vocabulary",
                    ask.capability
                ));
            }
            (media.as_slice() != routed.as_slice()).then(|| {
                format!(
                    "{} names {media:?} where the gate routes {routed:?}",
                    ask.capability
                )
            })
        })
        .collect()
}

#[test]
fn each_ask_names_the_media_the_gate_routes() {
    assert_eq!(faults(MANIFEST), Vec::<String>::new());
}

#[test]
fn an_ask_naming_no_media_is_a_fault() {
    assert_eq!(
        faults("[[ask]]\ncapability = \"library.curate\"\n"),
        ["library.curate names no media"]
    );
}

#[test]
fn an_ask_naming_a_medium_outside_the_vocabulary_is_a_fault() {
    assert_eq!(
        faults("[[ask]]\ncapability = \"library.curate\"\nmedia_types = [\"tv\", \"films\"]\n"),
        ["library.curate names films, outside the vocabulary"]
    );
}

#[test]
fn an_ask_naming_other_media_than_the_gate_routes_is_a_fault() {
    assert_eq!(
        faults("[[ask]]\ncapability = \"identity.source\"\nmedia_types = [\"tv\", \"music\"]\n"),
        ["identity.source names [\"tv\", \"music\"] where the gate routes [\"tv\", \"movies\"]"]
    );
}

#[test]
fn a_manifest_that_is_not_toml_is_a_fault() {
    assert_eq!(faults("[[ask"), ["the manifest is not TOML"]);
}
