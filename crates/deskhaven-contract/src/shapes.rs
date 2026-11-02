//! Fixture `expect.shape` names → "does the client mirror type parse these
//! exact bytes". Add an entry when an endpoint needs a new shape.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::mirror::{
    AbPeer, AuthBody, ClientUserPayload, ErrorBody, HeartbeatResponse, LoginOptions, LoginResponse,
    PagedResponse,
};

/// Parses a response body; `Err` is serde's message (with line/column).
pub type ShapeCheck = fn(&[u8]) -> Result<(), String>;

const SHAPES: &[(&str, ShapeCheck)] = &[
    ("AuthBody", object::<AuthBody>),
    ("ClientUserPayload", object::<ClientUserPayload>),
    ("ErrorBody", object::<ErrorBody>),
    ("HeartbeatResponse", object::<HeartbeatResponse>),
    ("LoginOptions", parse::<LoginOptions>),
    ("LoginResponse", object::<LoginResponse>),
    ("PagedResponse<AbPeer>", object::<PagedResponse<AbPeer>>),
    (
        "PagedResponse<ClientUserPayload>",
        object::<PagedResponse<ClientUserPayload>>,
    ),
];

/// From the raw bytes, as the client does: no detour through our own types.
fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<(), String> {
    serde_json::from_slice::<T>(body)
        .map(drop)
        .map_err(|err| err.to_string())
}

/// For struct mirrors. serde's derive also accepts a JSON array as a
/// positional struct, while the clients decode a map (`Map<String,
/// dynamic>`, `HashMap`) and reject it. Only the top level is checked: the
/// server serializes nested structs as objects as well.
fn object<T: DeserializeOwned>(body: &[u8]) -> Result<(), String> {
    let value: Value = serde_json::from_slice(body).map_err(|err| err.to_string())?;
    if !value.is_object() {
        return Err("expected a JSON object at the top level".to_owned());
    }
    parse::<T>(body)
}

pub fn lookup(name: &str) -> Option<ShapeCheck> {
    SHAPES
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, check)| *check)
}

pub fn names() -> impl Iterator<Item = &'static str> {
    SHAPES.iter().map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_up_by_exact_name() {
        let check = lookup("LoginOptions").unwrap();

        assert_eq!(check(b"[]"), Ok(()));
        assert!(check(b"{}").is_err());
        assert!(lookup("loginoptions").is_none());
    }

    #[test]
    fn struct_shapes_reject_positional_arrays() {
        let heartbeat = lookup("HeartbeatResponse").unwrap();
        let error = lookup("ErrorBody").unwrap();

        assert_eq!(heartbeat(b"{}"), Ok(()));
        assert!(heartbeat(b"[]").is_err());
        assert!(error(br#"["denied"]"#).is_err());
    }

    #[test]
    fn names_are_unique() {
        let mut all: Vec<_> = names().collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();

        assert_eq!(all.len(), count);
    }
}
