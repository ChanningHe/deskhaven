//! Compares a response with a fixture's `expect`.

use std::collections::BTreeMap;

use axum::http::HeaderMap;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::fixture::{BodyKind, Expect, HeaderMatch};
use crate::shapes;

/// Separator for a header sent more than once (RFC 9110 §5.3).
const HEADER_JOIN: &str = ", ";

/// What the router answered.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    /// The body as a client mirror type (multi-step tests).
    pub fn json<T: DeserializeOwned>(&self) -> serde_json::Result<T> {
        serde_json::from_slice(&self.body)
    }
}

/// Every mismatch, each a self-contained sentence. Empty = pass.
pub fn check(expect: &Expect, reply: &Reply) -> Vec<String> {
    let status = (reply.status != expect.status)
        .then(|| format!("status: expected {}, got {}", expect.status, reply.status));
    let body = match expect.body_kind {
        BodyKind::Empty => check_empty(&reply.body),
        BodyKind::Text => check_text(expect, &reply.body),
        BodyKind::Json => check_json(expect, &reply.body),
    };
    status
        .into_iter()
        .chain(check_headers(&expect.headers, &reply.headers))
        .chain(body)
        .collect()
}

/// Names were validated with the fixture; `HeaderMap` lookups ignore case.
fn check_headers(want: &BTreeMap<String, HeaderMatch>, got: &HeaderMap) -> Vec<String> {
    want.iter()
        .filter_map(|(name, rule)| {
            let name = name.to_ascii_lowercase();
            let values: Vec<_> = got
                .get_all(name.as_str())
                .iter()
                .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
                .collect();
            if values.is_empty() {
                return Some(format!("header {name}: missing"));
            }
            let value = values.join(HEADER_JOIN);
            match rule {
                HeaderMatch::Exact(exact) if *exact != value => {
                    Some(format!("header {name}: expected {exact:?}, got {value:?}"))
                }
                HeaderMatch::Contains(rule) if !value.contains(&rule.contains) => Some(format!(
                    "header {name}: expected to contain {:?}, got {value:?}",
                    rule.contains
                )),
                _ => None,
            }
        })
        .collect()
}

fn check_empty(body: &[u8]) -> Vec<String> {
    if body.is_empty() {
        return Vec::new();
    }
    vec![format!("body: expected empty, got {} bytes", body.len())]
}

fn check_text(expect: &Expect, body: &[u8]) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(body) else {
        return vec!["body: expected UTF-8 text".to_owned()];
    };
    match &expect.text {
        Some(want) if want != text => vec![format!("text: expected {want:?}, got {text:?}")],
        _ => Vec::new(),
    }
}

fn check_json(expect: &Expect, body: &[u8]) -> Vec<String> {
    let value: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(err) => return vec![format!("body: expected JSON ({err})")],
    };
    let shape = expect.shape.as_deref().and_then(|name| {
        let check = shapes::lookup(name)?;
        check(body)
            .err()
            .map(|err| format!("shape: the client's {name} does not parse the body: {err}"))
    });
    let subset = expect
        .json_subset
        .as_ref()
        .map(|want| subset(want, &value, ""))
        .unwrap_or_default();
    let absent = expect
        .json_absent
        .iter()
        .filter(|pointer| value.pointer(pointer).is_some())
        .map(|pointer| format!("json_absent: {pointer} is present"));
    shape.into_iter().chain(subset).chain(absent).collect()
}

/// By value: `1` equals `1.0`. Two integers compare exactly (serde_json's
/// `==` also tells `1` from `1.0`; f64 would merge integers above 2^53).
fn same_number(want: &serde_json::Number, got: &serde_json::Number) -> bool {
    let is_integer = |n: &serde_json::Number| n.is_i64() || n.is_u64();
    if is_integer(want) && is_integer(got) {
        return want == got;
    }
    want.as_f64().is_some_and(|w| got.as_f64() == Some(w))
}

/// Mismatches of `want` inside `got`, located by JSON pointer. Numbers
/// compare by value ([`same_number`]).
fn subset(want: &Value, got: &Value, at: &str) -> Vec<String> {
    match (want, got) {
        (Value::Object(want), Value::Object(got)) => want
            .iter()
            .flat_map(|(key, w)| {
                let here = format!("{at}/{key}");
                match got.get(key) {
                    Some(g) => subset(w, g, &here),
                    None => vec![format!("json_subset: {here} is missing")],
                }
            })
            .collect(),
        (Value::Array(want), Value::Array(got)) if want.len() == got.len() => want
            .iter()
            .zip(got)
            .enumerate()
            .flat_map(|(i, (w, g))| subset(w, g, &format!("{at}/{i}")))
            .collect(),
        (Value::Number(want), Value::Number(got)) if same_number(want, got) => Vec::new(),
        (want, got) if want == got => Vec::new(),
        (want, got) => {
            let at = if at.is_empty() { "(root)" } else { at };
            vec![format!("json_subset: {at}: expected {want}, got {got}")]
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use serde_json::json;

    use super::*;

    fn expect(value: Value) -> Expect {
        serde_json::from_value(value).unwrap()
    }

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn with_headers(headers: &[(&'static str, &'static str)]) -> Reply {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.append(*name, HeaderValue::from_static(value));
        }
        Reply {
            headers: map,
            ..reply(204, "")
        }
    }

    #[test]
    fn headers_match_exactly_or_by_substring_with_any_name_case() {
        let want = expect(json!({ "status": 204, "body_kind": "empty", "headers": {
            "Allow": "GET,HEAD",
            "CONTENT-TYPE": { "contains": "application/json" }
        } }));
        let got = with_headers(&[
            ("allow", "GET,HEAD"),
            ("content-type", "application/json; charset=utf-8"),
        ]);

        assert_eq!(check(&want, &got), Vec::<String>::new());
    }

    #[test]
    fn header_mismatches_are_reported() {
        let want = expect(json!({ "status": 204, "body_kind": "empty", "headers": {
            "allow": "GET",
            "content-type": { "contains": "json" },
            "x-missing": "1"
        } }));
        let got = with_headers(&[("allow", "GET,HEAD"), ("content-type", "text/plain")]);

        let problems = check(&want, &got);

        assert_eq!(
            problems,
            [
                r#"header allow: expected "GET", got "GET,HEAD""#,
                r#"header content-type: expected to contain "json", got "text/plain""#,
                "header x-missing: missing",
            ]
        );
    }

    #[test]
    fn repeated_headers_are_joined_in_order() {
        let want =
            expect(json!({ "status": 204, "body_kind": "empty", "headers": { "vary": "a, b" } }));

        assert!(check(&want, &with_headers(&[("vary", "a"), ("vary", "b")])).is_empty());
    }

    #[test]
    fn passes_matching_reply() {
        let want = expect(json!({
            "status": 200, "body_kind": "json", "shape": "LoginOptions", "json_subset": []
        }));

        assert_eq!(check(&want, &reply(200, "[]")), Vec::<String>::new());
    }

    #[test]
    fn reports_status_and_shape() {
        let want = expect(json!({ "status": 200, "body_kind": "json", "shape": "LoginOptions" }));

        let problems = check(&want, &reply(404, r#"{"error":"not found"}"#));

        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("expected 200, got 404"));
        assert!(problems[1].contains("LoginOptions"));
    }

    #[test]
    fn empty_and_text_bodies() {
        let empty = expect(json!({ "status": 200, "body_kind": "empty" }));
        let text = expect(json!({ "status": 200, "body_kind": "text", "text": "SYSINFO_UPDATED" }));

        assert_eq!(check(&empty, &reply(200, "")).len(), 0);
        assert_eq!(check(&empty, &reply(200, "{}")).len(), 1);
        assert_eq!(check(&text, &reply(200, "SYSINFO_UPDATED")).len(), 0);
        assert_eq!(check(&text, &reply(200, "ID_NOT_FOUND")).len(), 1);
    }

    #[test]
    fn subset_ignores_extra_keys_but_not_extra_items() {
        let got = json!({ "total": 1, "data": [{ "id": "1", "hash": "" }], "extra": true });

        assert!(subset(&json!({ "data": [{ "id": "1" }] }), &got, "").is_empty());
        assert_eq!(
            subset(&json!({ "data": [] }), &got, ""),
            vec![r#"json_subset: /data: expected [], got [{"hash":"","id":"1"}]"#]
        );
        assert_eq!(
            subset(&json!({ "nope": 1 }), &got, ""),
            vec!["json_subset: /nope is missing"]
        );
    }

    #[test]
    fn subset_compares_numbers_by_value() {
        let got = json!({ "total": 1, "ratio": 1.5, "big": 9_007_199_254_740_993_u64 });

        assert!(subset(&json!({ "total": 1.0, "ratio": 1.5 }), &got, "").is_empty());
        assert_eq!(subset(&json!({ "total": 2.0 }), &got, "").len(), 1);
        assert_eq!(subset(&json!({ "ratio": 1 }), &got, "").len(), 1);
        // Integers compare exactly (no detour through f64).
        assert_eq!(
            subset(&json!({ "big": 9_007_199_254_740_992_u64 }), &got, "").len(),
            1
        );
        assert_eq!(subset(&json!({ "total": "1" }), &got, "").len(), 1);
    }

    #[test]
    fn absent_pointers() {
        let want =
            expect(json!({ "status": 200, "body_kind": "json", "json_absent": ["/strategy"] }));

        assert_eq!(check(&want, &reply(200, r#"{"modified_at":1}"#)).len(), 0);
        assert_eq!(check(&want, &reply(200, r#"{"strategy":{}}"#)).len(), 1);
    }
}
