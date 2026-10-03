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
