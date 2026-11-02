//! Fixture file format (`tests/contract/fixtures/<endpoint>/<case>.json`).
//! Unknown keys are rejected so a typo cannot silently drop an assertion.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::http::Method;
use serde::Deserialize;
use serde_json::Value;

use crate::seeds::{SeedOutputs, Seeds};
use crate::shapes;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    /// Client commit the `source` lines were read at; must equal
    /// `tests/contract/UPSTREAM_COMMIT`.
    pub upstream_commit: String,
    /// `path:line` or `path:start-end` in the rustdesk repository, for each
    /// client line this fixture encodes.
    pub source: Vec<String>,
    pub description: String,
    /// Names from the seed registry, applied in order.
    #[serde(default)]
    pub seed: Vec<String>,
    pub request: Request,
    pub expect: Expect,
}

/// Sent exactly as written: no implicit headers besides `content-length`
/// (set from the body when a body is sent or the method carries one).
///
/// `${name}` in `path`, header values and body strings (text, or any string
/// inside a JSON body) is replaced by the seed output `name` before sending;
/// an unknown name fails the fixture. There is no escape for a literal `${`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub method: String,
    /// Path and query string as they appear on the wire.
    pub path: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Body,
    /// TCP peer (`ConnectInfo`); defaults to [`crate::app::DEFAULT_PEER`].
    #[serde(default)]
    pub peer: Option<SocketAddr>,
}

/// `"empty"`, `{"json": <value>}` (serialized compactly) or `{"text": "..."}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    #[default]
    Empty,
    Json(Value),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Zero bytes (e.g. AB writes and audit: 200 + empty body = success).
    Empty,
    /// Valid JSON.
    Json,
    /// UTF-8 text (e.g. sysinfo's `SYSINFO_UPDATED`).
    Text,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    pub status: u16,
    pub body_kind: BodyKind,
    /// Client mirror type the body must parse as (see [`crate::shapes`]).
    #[serde(default)]
    pub shape: Option<String>,
    /// Objects: listed keys must match (others ignored); arrays: same length,
    /// element-wise; scalars: equal.
    #[serde(default)]
    pub json_subset: Option<Value>,
    /// JSON pointers (`/strategy`, `/data/0/hash`) that must not exist.
    #[serde(default)]
    pub json_absent: Vec<String>,
    /// Exact body text.
    #[serde(default)]
    pub text: Option<String>,
}

impl Fixture {
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|err| format!("invalid fixture: {err}"))
    }

    /// Everything wrong with the fixture itself, before it is run.
    pub fn problems(&self, upstream_commit: &str, seeds: &Seeds) -> Vec<String> {
        [
            self.provenance_problems(upstream_commit),
            self.request.problems(),
            self.expect.problems(),
            self.seed
                .iter()
                .filter(|name| seeds.lookup(name).is_none())
                .map(|name| format!("unknown seed {name:?}"))
                .collect(),
        ]
        .concat()
    }

    fn provenance_problems(&self, upstream_commit: &str) -> Vec<String> {
        let commit = (self.upstream_commit != upstream_commit).then(|| {
            format!(
                "upstream_commit {:?} != UPSTREAM_COMMIT {upstream_commit:?}: re-verify the \
                 source lines at the tracked commit, then update the fixture",
                self.upstream_commit
            )
        });
        let no_source = self
            .source
            .is_empty()
            .then(|| "source must cite at least one client file:line".to_owned());
        let no_description = self
            .description
            .trim()
            .is_empty()
            .then(|| "description must not be empty".to_owned());
        let bad_sources = self
            .source
            .iter()
            .filter(|s| !is_file_line(s))
            .map(|s| format!("source {s:?} is not `path:line` or `path:start-end`"));
        commit
            .into_iter()
            .chain(no_source)
            .chain(no_description)
            .chain(bad_sources)
            .collect()
    }
}

impl Request {
    /// No headers, no body, default peer (multi-step tests set the rest).
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_owned(),
            path: path.to_owned(),
            headers: BTreeMap::new(),
            body: Body::Empty,
            peer: None,
        }
    }

    /// A copy with every `${name}` replaced from `outputs`.
    pub fn resolve(&self, outputs: &SeedOutputs) -> Result<Self, String> {
        let headers = self
            .headers
            .iter()
            .map(|(name, value)| Ok((name.clone(), substitute(value, outputs)?)))
            .collect::<Result<_, String>>()?;
        let body = match &self.body {
            Body::Empty => Body::Empty,
            Body::Json(value) => Body::Json(substitute_json(value, outputs)?),
            Body::Text(text) => Body::Text(substitute(text, outputs)?),
        };
        Ok(Self {
            method: self.method.clone(),
            path: substitute(&self.path, outputs)?,
            headers,
            body,
            peer: self.peer,
        })
    }

    fn problems(&self) -> Vec<String> {
        let method = Method::from_bytes(self.method.as_bytes())
            .is_err()
            .then(|| format!("invalid method {:?}", self.method));
        let path = (!self.path.starts_with('/'))
            .then(|| format!("path {:?} must start with '/'", self.path));
        method.into_iter().chain(path).collect()
    }
}

impl Expect {
    fn problems(&self) -> Vec<String> {
        let is_json = self.body_kind == BodyKind::Json;
        let json_only = [
            ("shape", self.shape.is_some()),
            ("json_subset", self.json_subset.is_some()),
            ("json_absent", !self.json_absent.is_empty()),
        ];
        let misplaced = json_only
            .into_iter()
            .filter(|(_, set)| *set && !is_json)
            .map(|(key, _)| format!("{key} requires body_kind \"json\""));
        let text = (self.text.is_some() && self.body_kind != BodyKind::Text)
            .then(|| "text requires body_kind \"text\"".to_owned());
        let shape = self
            .shape
            .as_deref()
            .filter(|name| shapes::lookup(name).is_none())
            .map(|name| {
                let known: Vec<_> = shapes::names().collect();
                format!("unknown shape {name:?} (known: {})", known.join(", "))
            });
        let pointers = self
            .json_absent
            .iter()
            .filter(|p| !p.starts_with('/'))
            .map(|p| format!("json_absent entry {p:?} must be a JSON pointer (\"/key\")"));
        misplaced.chain(text).chain(shape).chain(pointers).collect()
    }
}

const PLACEHOLDER_OPEN: &str = "${";
const PLACEHOLDER_CLOSE: char = '}';

/// `text` with each `${name}` replaced by `outputs[name]`.
fn substitute(text: &str, outputs: &SeedOutputs) -> Result<String, String> {
    let mut resolved = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(PLACEHOLDER_OPEN) {
        let after = &rest[start + PLACEHOLDER_OPEN.len()..];
        let end = after
            .find(PLACEHOLDER_CLOSE)
            .ok_or_else(|| format!("unterminated placeholder in {text:?}"))?;
        let name = &after[..end];
        let value = outputs.get(name).ok_or_else(|| {
            let known: Vec<_> = outputs.keys().map(String::as_str).collect();
            format!(
                "placeholder ${{{name}}} has no seed output (outputs: [{}])",
                known.join(", ")
            )
        })?;
        resolved.push_str(&rest[..start]);
        resolved.push_str(value);
        rest = &after[end + 1..];
    }
    resolved.push_str(rest);
    Ok(resolved)
}

/// Substitutes inside every string of a JSON value (keys stay as written).
fn substitute_json(value: &Value, outputs: &SeedOutputs) -> Result<Value, String> {
    Ok(match value {
        Value::String(text) => Value::String(substitute(text, outputs)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_json(item, outputs))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), substitute_json(item, outputs)?)))
                .collect::<Result<_, String>>()?,
        ),
        scalar => scalar.clone(),
    })
}

/// `path:12` or `path:12-30`, path without whitespace.
fn is_file_line(source: &str) -> bool {
    let Some((path, lines)) = source.rsplit_once(':') else {
        return false;
    };
    let is_number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let lines_ok = match lines.split_once('-') {
        Some((start, end)) => is_number(start) && is_number(end),
        None => is_number(lines),
    };
    !path.is_empty() && !path.contains(char::is_whitespace) && lines_ok
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const COMMIT: &str = "abc123";

    fn fixture(patch: Value) -> Fixture {
        let mut base = json!({
            "upstream_commit": COMMIT,
            "source": ["flutter/lib/models/user_model.dart:240-264"],
            "description": "d",
            "request": { "method": "GET", "path": "/api/login-options" },
            "expect": { "status": 200, "body_kind": "json", "shape": "LoginOptions", "json_subset": [] }
        });
        merge(&mut base, patch);
        Fixture::parse(&base.to_string()).unwrap()
    }

    fn merge(base: &mut Value, patch: Value) {
        match (base, patch) {
            (Value::Object(base), Value::Object(patch)) => {
                for (key, value) in patch {
                    merge(base.entry(key).or_insert(Value::Null), value);
                }
            }
            (base, patch) => *base = patch,
        }
    }

    #[test]
    fn valid_fixture_has_no_problems() {
        assert_eq!(
            fixture(json!({})).problems(COMMIT, &Seeds::builtin()),
            Vec::<String>::new()
        );
    }

    #[test]
    fn body_variants_parse() {
        let empty = fixture(json!({ "request": { "body": "empty" } }));
        let json = fixture(json!({ "request": { "body": { "json": { "id": "1" } } } }));
        let text = fixture(json!({ "request": { "body": { "text": "x" } } }));

        assert!(matches!(empty.request.body, Body::Empty));
        assert!(matches!(json.request.body, Body::Json(_)));
        assert!(matches!(text.request.body, Body::Text(_)));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let text = json!({
            "upstream_commit": COMMIT, "source": ["a:1"], "description": "d",
            "request": { "method": "GET", "path": "/" },
            "expect": { "status": 200, "body_kind": "empty", "shapee": "X" }
        });

        assert!(Fixture::parse(&text.to_string()).is_err());
    }

    #[test]
    fn provenance_is_enforced() {
        let stale = fixture(json!({ "upstream_commit": "old" }));
        let bad = fixture(json!({ "source": ["user_model.dart", "a b:1", "x:1-"] }));
        let none = fixture(json!({ "source": [] }));

        assert_eq!(stale.problems(COMMIT, &Seeds::builtin()).len(), 1);
        assert_eq!(bad.problems(COMMIT, &Seeds::builtin()).len(), 3);
        assert_eq!(none.problems(COMMIT, &Seeds::builtin()).len(), 1);
    }

    #[test]
    fn expectations_must_fit_the_body_kind() {
        let text_with_shape = fixture(json!({ "expect": { "body_kind": "text" } }));
        let unknown_shape = fixture(json!({ "expect": { "shape": "Nope" } }));
        let bad_pointer = fixture(json!({ "expect": { "json_absent": ["strategy"] } }));
        let text_on_json = fixture(json!({ "expect": { "text": "x" } }));

        // shape + json_subset are both misplaced on a text body.
        assert_eq!(text_with_shape.problems(COMMIT, &Seeds::builtin()).len(), 2);
        assert!(unknown_shape.problems(COMMIT, &Seeds::builtin())[0].contains("LoginOptions"));
        assert_eq!(bad_pointer.problems(COMMIT, &Seeds::builtin()).len(), 1);
        assert_eq!(text_on_json.problems(COMMIT, &Seeds::builtin()).len(), 1);
    }

    #[test]
    fn request_and_seed_problems() {
        let bad = fixture(json!({
            "seed": ["no-such-seed"],
            "request": { "method": "G ET", "path": "api/x" }
        }));

        assert_eq!(bad.problems(COMMIT, &Seeds::builtin()).len(), 3);
    }

    fn outputs() -> SeedOutputs {
        SeedOutputs::from([
            ("token".to_owned(), "t1".to_owned()),
            ("id".to_owned(), "42".to_owned()),
        ])
    }

    #[test]
    fn substitutes_every_placeholder() {
        let resolved = substitute("/api/x?id=${id}&t=${token}${id}", &outputs());

        assert_eq!(resolved.as_deref(), Ok("/api/x?id=42&t=t142"));
        assert_eq!(
            substitute("no placeholder $ { }", &outputs()).as_deref(),
            Ok("no placeholder $ { }")
        );
    }

    #[test]
    fn unknown_or_unterminated_placeholders_are_errors() {
        let unknown = substitute("Bearer ${nope}", &outputs()).unwrap_err();
        let open = substitute("Bearer ${token", &outputs()).unwrap_err();

        assert!(
            unknown.contains("${nope}") && unknown.contains("id, token"),
            "{unknown}"
        );
        assert!(open.contains("unterminated"), "{open}");
    }

    #[test]
    fn resolve_covers_path_headers_and_json_strings_only() {
        let request = fixture(json!({ "request": {
            "path": "/api/x/${id}",
            "headers": { "authorization": "Bearer ${token}" },
            "body": { "json": { "${id}": ["${token}", 1, { "n": "${id}" }] } }
        } }))
        .request;

        let resolved = request.resolve(&outputs()).unwrap();

        assert_eq!(resolved.path, "/api/x/42");
        assert_eq!(resolved.headers["authorization"], "Bearer t1");
        let Body::Json(body) = resolved.body else {
            panic!("json body expected");
        };
        assert_eq!(body, json!({ "${id}": ["t1", 1, { "n": "42" }] }));
        // The original is untouched.
        assert_eq!(request.path, "/api/x/${id}");
    }
}
