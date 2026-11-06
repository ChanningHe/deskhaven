use std::collections::BTreeMap;

use deskhaven_contract::fixture::{Body, Fixture};
use deskhaven_contract::mirror::HeartbeatResponse;
use deskhaven_contract::runner::contract_dir;

use super::*;

const FROM: Sender<'static> = Sender {
    id: "100000007",
    uuid_b64: "bG9hZGdlbi0wMDAwMDAwMDA3",
};

/// The JSON request body of a contract fixture.
fn fixture_body(endpoint: &str, case: &str) -> Map<String, Value> {
    let path = contract_dir()
        .join("fixtures")
        .join(endpoint)
        .join(format!("{case}.json"));
    let fixture = Fixture::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    match fixture.request.body {
        Body::Json(Value::Object(map)) => map,
        other => panic!("{}: not a JSON object body: {other:?}", path.display()),
    }
}

/// Key → JSON type; a `${int:...}` placeholder is a number, any other `${...}` a string.
fn shape(map: &Map<String, Value>) -> BTreeMap<String, &'static str> {
    let kind = |value: &Value| match value {
        Value::String(s) if s.starts_with("${int:") => "number",
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    map.iter().map(|(k, v)| (k.clone(), kind(v))).collect()
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

#[test]
fn heartbeat_has_the_fixture_shape_with_and_without_conns() {
    let quiet = object(heartbeat(FROM, &[], 1_730_000_100));
    assert_eq!(
        shape(&quiet),
        shape(&fixture_body("heartbeat", "current-strategy"))
    );
    assert_eq!(quiet["id"], FROM.id);
    assert_eq!(quiet["uuid"], FROM.uuid_b64);
    assert_eq!(quiet["modified_at"], 1_730_000_100);
    // `get_version_number("1.4.1")`, as the client computes it (the server ignores `ver`).
    assert_eq!(quiet["ver"], 1_004_010);

    let live = object(heartbeat(FROM, &[3], 0));
    assert_eq!(
        shape(&live),
        shape(&fixture_body("heartbeat", "disconnect"))
    );
    assert_eq!(live["conns"], json!([3]));
}

#[test]
fn sysinfo_has_the_fixture_shape() {
    let body = object(sysinfo(FROM, 7));

    assert_eq!(shape(&body), shape(&fixture_body("sysinfo", "registered")));
    assert_eq!(body["version"], CLIENT_VERSION);
    assert_eq!(body["id"], FROM.id);
}

#[test]
fn audit_records_have_the_fixture_shapes() {
    let call = |step, session_id| AuditCall {
        step,
        conn_id: 4,
        session_id,
    };
    let new = object(audit(FROM, &call(AuditStep::New, 0), "n1"));
    let authorized = object(audit(FROM, &call(AuditStep::Authorized, 99), "n2"));
    let close = object(audit(FROM, &call(AuditStep::Close, 99), "n3"));

    assert_eq!(shape(&new), shape(&fixture_body("audit-conn", "new")));
    assert_eq!(shape(&close), shape(&fixture_body("audit-conn", "close")));
    // A connection without a second factor leaves `two_factor` out (03 §2.3).
    let mut expected = shape(&fixture_body("audit-conn", "authorized"));
    expected.remove("two_factor");
    assert_eq!(shape(&authorized), expected);
    assert_eq!(
        (new["session_id"].clone(), new["nonce"].clone()),
        (json!(0), json!("n1"))
    );
    assert_eq!(close["session_id"], 99);
}

#[test]
fn heartbeat_reply_reads_sysinfo_presence_and_modified_at() {
    let body = br#"{"sysinfo": true, "modified_at": 5, "strategy": {"config_options": {}}}"#;
    // The contract's client mirror accepts what the simulator accepts.
    let mirror: HeartbeatResponse = serde_json::from_slice(body).unwrap();
    assert_eq!(mirror.modified_at, Some(5));

    let reply = heartbeat_reply(200, body).unwrap();

    assert_eq!(
        reply,
        HeartbeatReply {
            sysinfo: true,
            modified_at: Some(5)
        }
    );
    assert_eq!(
        heartbeat_reply(200, b"{}").unwrap(),
        HeartbeatReply::default()
    );
}

#[test]
fn heartbeat_reply_failures() {
    assert_eq!(
        heartbeat_reply(404, br#"{"error":"ID_NOT_FOUND"}"#),
        Err(Failure::Status(404))
    );
    assert!(matches!(
        heartbeat_reply(200, b"[1]"),
        Err(Failure::Body(_))
    ));
    assert!(matches!(
        heartbeat_reply(200, b"nope"),
        Err(Failure::Body(_))
    ));
}

#[test]
fn sysinfo_reply_is_classified_like_the_client() {
    assert_eq!(
        sysinfo_reply(200, b"SYSINFO_UPDATED"),
        (SysinfoOutcome::Updated, Ok(()))
    );
    let (outcome, result) = sysinfo_reply(200, b"ID_NOT_FOUND");
    assert_eq!(outcome, SysinfoOutcome::IdNotFound);
    assert!(matches!(result, Err(Failure::Body(_))));
    assert_eq!(
        sysinfo_reply(429, b""),
        (SysinfoOutcome::Other, Err(Failure::Status(429)))
    );
}

#[test]
fn audit_reply_needs_an_empty_2xx() {
    assert_eq!(audit_reply(200, b""), Ok(()));
    assert!(matches!(audit_reply(200, b"x"), Err(Failure::Body(_))));
    assert_eq!(audit_reply(404, b""), Err(Failure::Status(404)));
}
