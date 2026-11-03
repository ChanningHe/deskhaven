//! Fixture `expect.shape` names → "does the client mirror type parse these
//! exact bytes". Add an entry when an endpoint needs a new shape.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::mirror::{
    AbPeer, AbPersonal, AbProfile, AbSettings, AbTag, AuthBody, ClientUserPayload, DeployResponse,
    ErrorBody, HbbData, HeartbeatResponse, LegacyAbReply, LoginOptions, LoginResponse, OidcAuthUrl,
    PagedResponse, SecondFactorChallenge,
};

/// Parses a response body; `Err` is serde's message (with line/column).
pub type ShapeCheck = fn(&[u8]) -> Result<(), String>;

/// Types the Rust client reads through `HbbHttpResponse`, and the address book
/// bodies Flutter refuses when they contain an `error` key
/// (`flutter/lib/models/ab_model.dart:244-247`), are registered as `HbbData<T>`
/// (object only, no top-level `error` key).
const SHAPES: &[(&str, ShapeCheck)] = &[
    ("AuthBody", parse::<HbbData<AuthBody>>),
    ("ClientUserPayload", object::<ClientUserPayload>),
    ("DeployResponse", object::<DeployResponse>),
    ("ErrorBody", object::<ErrorBody>),
    ("HeartbeatResponse", object::<HeartbeatResponse>),
    ("LoginOptions", parse::<LoginOptions>),
    ("LoginResponse", object::<LoginResponse>),
    ("OidcAuthUrl", parse::<HbbData<OidcAuthUrl>>),
    ("SecondFactorChallenge", object::<SecondFactorChallenge>),
    ("AbPersonal", parse::<HbbData<AbPersonal>>),
    ("AbSettings", parse::<HbbData<AbSettings>>),
    (
        "PagedResponse<AbPeer>",
        parse::<HbbData<PagedResponse<AbPeer>>>,
    ),
    (
        "PagedResponse<AbProfile>",
        parse::<HbbData<PagedResponse<AbProfile>>>,
    ),
    ("Vec<AbTag>", parse::<Vec<AbTag>>),
    ("LegacyAb", parse::<Option<LegacyAbReply>>),
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

    const USER: &str = r#"{"name":"alice","info":{}}"#;

    fn auth_body(extra: &str) -> Vec<u8> {
        format!(r#"{{"access_token":"t","type":"access_token","user":{USER}{extra}}}"#).into_bytes()
    }

    #[test]
    fn hbb_shapes_reject_any_top_level_error_key() {
        // HbbHttpResponse::parse: an `error` key, whatever its value, makes
        // the body an error and the data is never read.
        let auth = lookup("AuthBody").unwrap();
        let oidc = lookup("OidcAuthUrl").unwrap();
        let url = r#"{"code":"c1","url":"https://idp.example/auth?x=1""#;

        assert_eq!(auth(&auth_body("")), Ok(()));
        for error in [r#""""#, r#""denied""#, "null", "1", "{}"] {
            let extra = format!(r#","error":{error}"#);
            assert!(
                auth(&auth_body(&extra)).is_err(),
                "AuthBody with error {error}"
            );
            let body = format!("{url}{extra}}}");
            assert!(
                oidc(body.as_bytes()).is_err(),
                "OidcAuthUrl with error {error}"
            );
        }
        assert_eq!(oidc(format!("{url}}}").as_bytes()), Ok(()));
    }

    #[test]
    fn oidc_auth_url_requires_code_and_an_absolute_url() {
        let oidc = lookup("OidcAuthUrl").unwrap();

        assert!(oidc(br#"{"code":"c1","url":"/relative"}"#).is_err());
        assert!(oidc(br#"{"url":"https://idp.example/"}"#).is_err());
        assert!(oidc(br#"[["c1","https://idp.example/"]]"#).is_err());
    }

    #[test]
    fn error_body_takes_the_error_only_as_a_string() {
        let error = lookup("ErrorBody").unwrap();

        assert_eq!(error(br#"{"error":"denied"}"#), Ok(()));
        for body in [
            r#"{"error":null}"#,
            r#"{"error":1}"#,
            r#"{"error":{}}"#,
            r#"{"error":[]}"#,
        ] {
            assert!(error(body.as_bytes()).is_err(), "{body}");
        }
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
