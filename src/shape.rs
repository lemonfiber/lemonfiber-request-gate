//! The kinds of value the list names, and how each is read from what was sent.
//!
//! A named parameter or field sent in another kind refuses the call; one not sent is
//! left out; anything not named is dropped.

use serde_json::{Map, Value};

use crate::asked::{Asked, Query};
use crate::upstream::Stop;

/// The kind of value a parameter or a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A whole number, never negative.
    Integer,
    /// Text.
    Text,
    /// `true` or `false`.
    Flag,
    /// A list of whole numbers, never negative.
    Integers,
}

impl Shape {
    /// `value` as this shape, where it is one.
    pub(crate) fn of(self, value: &Value) -> Option<Value> {
        match self {
            Self::Integer => value.as_u64().map(Value::from),
            Self::Text => value.as_str().map(Value::from),
            Self::Flag => value.as_bool().map(Value::from),
            Self::Integers => value
                .as_array()?
                .iter()
                .map(Value::as_u64)
                .collect::<Option<Vec<_>>>()
                .map(Value::from),
        }
    }

    /// The parameter text `sent` as this shape, spelled as it is sent upstream.
    fn sent(self, sent: &str) -> Option<String> {
        match self {
            Self::Integer => sent.parse::<u64>().ok().map(|number| number.to_string()),
            Self::Text => Some(sent.to_owned()),
            Self::Flag => ["true", "false"]
                .into_iter()
                .find(|flag| flag.eq_ignore_ascii_case(sent))
                .map(str::to_owned),
            Self::Integers => None,
        }
    }
}

/// The parameters `named` in `query`, each in its shape.
pub(crate) fn parameters(
    query: &Query,
    named: &[(&'static str, Shape)],
) -> Result<Vec<(&'static str, String)>, Stop> {
    let mut kept = Vec::new();
    for &(name, shape) in named {
        if let Some(sent) = query.one(name).map_err(|_| Stop::Refused)? {
            kept.push((name, shape.sent(sent).ok_or(Stop::Refused)?));
        }
    }
    Ok(kept)
}

/// The fields `named` in `from`, each in its shape.
pub(crate) fn fields(
    from: &Map<String, Value>,
    named: &[(&'static str, Shape)],
) -> Result<Map<String, Value>, Stop> {
    let mut kept = Map::new();
    for &(name, shape) in named {
        if let Some(value) = from.get(name) {
            kept.insert(name.to_owned(), shape.of(value).ok_or(Stop::Refused)?);
        }
    }
    Ok(kept)
}

/// The call's body, which must be one JSON object.
pub(crate) fn object(asked: &Asked) -> Result<Map<String, Value>, Stop> {
    asked
        .body
        .as_deref()
        .and_then(|body| serde_json::from_slice::<Value>(body).ok())
        .and_then(|body| match body {
            Value::Object(fields) => Some(fields),
            _ => None,
        })
        .ok_or(Stop::Refused)
}

/// The object `name` holds in `from`, where it holds one; refused where it holds
/// anything else.
pub(crate) fn nested<'a>(
    from: &'a Map<String, Value>,
    name: &str,
) -> Result<Option<&'a Map<String, Value>>, Stop> {
    from.get(name)
        .map(|value| value.as_object().ok_or(Stop::Refused))
        .transpose()
}

/// The field `name` of `from`, which must be there and in `shape`.
pub(crate) fn required(from: &Map<String, Value>, name: &str, shape: Shape) -> Result<Value, Stop> {
    from.get(name)
        .and_then(|value| shape.of(value))
        .ok_or(Stop::Refused)
}

#[cfg(test)]
mod tests;
