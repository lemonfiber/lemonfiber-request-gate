use serde_json::{json, Map, Value};

use super::{fields, nested, parameters, required, Shape};
use crate::asked::Query;
use crate::upstream::Stop;

#[test]
fn each_shape_reads_only_its_own_kind_of_value() {
    assert_eq!(Shape::Integer.of(&json!(7)), Some(json!(7)));
    assert_eq!(Shape::Integer.of(&json!(-7)), None);
    assert_eq!(Shape::Integer.of(&json!("7")), None);
    assert_eq!(Shape::Text.of(&json!("a")), Some(json!("a")));
    assert_eq!(Shape::Text.of(&json!(1)), None);
    assert_eq!(Shape::Flag.of(&json!(true)), Some(json!(true)));
    assert_eq!(Shape::Flag.of(&json!("true")), None);
    assert_eq!(Shape::Integers.of(&json!([1, 2])), Some(json!([1, 2])));
    assert_eq!(Shape::Integers.of(&json!([1, "2"])), None);
    assert_eq!(Shape::Integers.of(&json!(1)), None);
}

#[test]
fn a_parameter_is_kept_in_its_shape_and_spelled_as_it_is_sent_upstream() {
    let query = Query::default();
    assert_eq!(
        parameters(&query, &[("seriesId", Shape::Integer)]),
        Ok(Vec::new())
    );

    let sent = |text: &str, shape| {
        let query = crate::asked::Asked::new(
            axum::http::Method::GET,
            &format!("/r?p={text}").parse().unwrap_or_default(),
            None,
        )
        .query;
        parameters(&query, &[("p", shape)])
    };
    assert_eq!(sent("007", Shape::Integer), Ok(vec![("p", "7".to_owned())]));
    assert_eq!(
        sent("False", Shape::Flag),
        Ok(vec![("p", "false".to_owned())])
    );
    assert_eq!(
        sent("a%20b", Shape::Text),
        Ok(vec![("p", "a b".to_owned())])
    );
    assert_eq!(sent("1", Shape::Integers), Err(Stop::Refused));
    assert_eq!(sent("x", Shape::Integer), Err(Stop::Refused));
}

#[test]
fn a_field_is_kept_in_its_shape_and_one_not_named_is_dropped() {
    let mut sent = Map::new();
    sent.insert("kept".to_owned(), json!(1));
    sent.insert("dropped".to_owned(), json!(2));

    let kept = fields(&sent, &[("kept", Shape::Integer), ("absent", Shape::Text)]);

    assert_eq!(kept.map(Value::Object), Ok(json!({ "kept": 1 })));
    assert_eq!(fields(&sent, &[("kept", Shape::Text)]), Err(Stop::Refused));
    assert_eq!(required(&sent, "kept", Shape::Integer), Ok(json!(1)));
    assert_eq!(
        required(&sent, "absent", Shape::Integer),
        Err(Stop::Refused)
    );
    assert_eq!(nested(&sent, "absent"), Ok(None));
    assert_eq!(nested(&sent, "kept"), Err(Stop::Refused));
}
