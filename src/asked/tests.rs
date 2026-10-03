use axum::http::{HeaderMap, HeaderValue, Method, Uri};

use super::{Asked, Fields, Query, Twice};

fn asked(uri: &'static str, authorisation: Option<&'static str>) -> Asked {
    let mut headers = HeaderMap::new();
    if let Some(value) = authorisation {
        headers.insert("authorization", HeaderValue::from_static(value));
    }
    Asked::new(Method::GET, &Uri::from_static(uri), &headers, None)
}

#[test]
fn the_route_is_the_first_segment_and_the_rest_is_matched_in_lower_case() {
    let asked = asked("/Sonarr/API/v3/QualityProfile?apikey=t", None);

    assert_eq!(asked.route, "Sonarr");
    assert_eq!(asked.segments(), ["api", "v3", "qualityprofile"]);
    assert_eq!(asked.path, "/Sonarr/API/v3/QualityProfile");
}

#[test]
fn the_root_names_no_route() {
    let asked = asked("/", None);

    assert_eq!(asked.route, "");
    assert!(asked.segments().is_empty());
}

#[test]
fn a_parameter_is_matched_without_regard_to_case_and_refused_when_sent_twice() {
    let query = Query::parse(Some("ApiKey=a&term=The%20Expanse&x=1&x=2"));

    assert_eq!(query.one("apikey"), Ok(Some("a")));
    assert_eq!(query.one("term"), Ok(Some("The Expanse")));
    assert_eq!(query.one("missing"), Ok(None));
    assert_eq!(query.one("x"), Err(Twice));
    assert_eq!(Query::parse(None), Query::default());
}

#[test]
fn the_media_browser_fields_are_read_from_its_header_and_no_other() {
    let read = asked(
        "/jellyfin/Users",
        Some(r#"MediaBrowser Client="Seerr", DeviceId="Qk9U", broken, Token="t""#),
    );

    assert_eq!(read.authorisation.get("token"), Some("t"));
    assert_eq!(read.authorisation.get("DeviceId"), Some("Qk9U"));
    assert_eq!(read.authorisation.get("broken"), None);
    assert_eq!(
        asked("/jellyfin/Users", Some(r#"mediabrowser Token="t""#))
            .authorisation
            .get("Token"),
        Some("t")
    );
    assert_eq!(
        asked("/jellyfin/Users", Some("Bearer t")).authorisation,
        Fields::default()
    );
    assert_eq!(
        asked("/jellyfin/Users", Some("Media")).authorisation,
        Fields::default()
    );
    assert_eq!(
        asked("/jellyfin/Users", None).authorisation,
        Fields::default()
    );
}
