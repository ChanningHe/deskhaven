//! Compares a response with a fixture's `expect`.

use serde_json::Value;

use crate::fixture::{BodyKind, Expect};
use crate::shapes;

/// What the router answered.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
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
    status.into_iter().chain(body).collect()
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

/// Mismatches of `want` inside `got`, located by JSON pointer.
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
        (want, got) if want == got => Vec::new(),
        (want, got) => {
            let at = if at.is_empty() { "(root)" } else { at };
            vec![format!("json_subset: {at}: expected {want}, got {got}")]
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn expect(value: Value) -> Expect {
        serde_json::from_value(value).unwrap()
    }

    fn reply(status: u16, body: &str) -> Reply {
        Reply {
            status,
            body: body.as_bytes().to_vec(),
        }
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
    fn absent_pointers() {
        let want =
            expect(json!({ "status": 200, "body_kind": "json", "json_absent": ["/strategy"] }));

        assert_eq!(check(&want, &reply(200, r#"{"modified_at":1}"#)).len(), 0);
        assert_eq!(check(&want, &reply(200, r#"{"strategy":{}}"#)).len(), 1);
    }
}
