use std::path::PathBuf;

use super::Settings;

#[test]
fn the_stack_default_holds_where_nothing_is_set() {
    assert_eq!(Settings::from(|_| None).config, PathBuf::from("/config"));
}

#[test]
fn a_set_variable_takes_the_place_of_its_default() {
    let settings = Settings::from(|name| {
        (name == "LEMONFIBER_REQUEST_GATE_CONFIG").then(|| "/elsewhere".to_owned())
    });

    assert_eq!(settings.config, PathBuf::from("/elsewhere"));
}

#[test]
fn plex_tv_is_reached_through_the_egress_the_stack_names_and_directly_otherwise() {
    assert_eq!(Settings::from(|_| None).plex_tv_through, None);
    let named = |value: &'static str| {
        Settings::from(move |name| {
            (name == "LEMONFIBER_REQUEST_GATE_PLEX_TV_THROUGH").then(|| value.to_owned())
        })
        .plex_tv_through
    };
    assert_eq!(
        named("http://request-gate-egress:3128").as_deref(),
        Some("http://request-gate-egress:3128")
    );
    assert_eq!(named(""), None);
}
