//! Parsing of the audit bodies, as the client builds them.

use serde_json::json;

use super::*;

fn bytes(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

fn base(extra: Value) -> Value {
    let mut body = json!({ "id": "123456789", "uuid": "dXVpZA==", "conn_id": 7,
                           "session_id": 0, "nonce": "n-1" });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    body
}

fn record(extra: Value) -> (Envelope, ConnReport) {
    match conn(&bytes(base(extra))).unwrap() {
        ConnMessage::Record(envelope, report) => (envelope, report),
        ConnMessage::Note(note) => panic!("{note:?}"),
    }
}

#[test]
fn parses_the_three_connection_records() {
    let (envelope, new) = record(json!({ "action": "new", "ip": "::ffff:10.0.0.1",
                                         "conn_audit_ref": "x" }));
    let (_, auth) = record(json!({ "peer": ["987", "bob"], "type": 4, "two_factor": 2,
                                   "session_id": "55" }));
    let (_, close) = record(json!({ "action": "close" }));

    assert_eq!(envelope.nonce.as_deref(), Some("n-1"));
    assert_eq!(
        new.event,
        ConnEvent::New {
            ip: Some("::ffff:10.0.0.1".parse().unwrap())
        }
    );
    assert!(new.session_id.is_unbound());
    let ConnEvent::Authorized(a) = auth.event else {
        panic!()
    };
    assert_eq!((a.peer_id.as_str(), a.peer_name.as_str()), ("987", "bob"));
    assert_eq!(
        (a.conn_type, a.primary_auth, a.two_factor),
        (4, None, Some(2))
    );
    assert_eq!(auth.session_id.as_str(), "55");
    assert_eq!(close.event, ConnEvent::Close);
}

#[test]
fn a_note_without_conn_id_and_action_is_the_controllers() {
    let body = json!({ "id": "123456789", "session_id": 99, "note": "hi" });

    let ConnMessage::Note(note) = conn(&bytes(body)).unwrap() else {
        panic!()
    };

    assert_eq!(
        (note.rustdesk_id.as_str(), note.note.as_str()),
        ("123456789", "hi")
    );
    assert_eq!(note.session_id.as_str(), "99");
    let unbound = json!({ "id": "123456789", "session_id": 0, "note": "hi" });
    assert!(conn(&bytes(unbound)).is_err());
}

#[test]
fn rejects_unusable_connection_records() {
    let bad = [
        base(json!({})),
        base(json!({ "action": "open" })),
        base(json!({ "action": "new", "ip": "not-an-ip" })),
        base(json!({ "peer": ["987", "bob"] })),
        base(json!({ "peer": "987", "type": 0 })),
        base(json!({ "action": "close", "session_id": 1.5 })),
        base(json!({ "action": "close", "session_id": null })),
        base(json!({ "action": "close", "conn_id": "7" })),
        base(json!({ "action": "close", "uuid": "" })),
        base(json!({ "action": "close", "nonce": "" })),
        base(json!({ "action": "close", "nonce": "a b" })),
        base(json!({ "action": "close", "nonce": "n".repeat(NONCE_MAX_LEN + 1) })),
        base(json!({ "action": "close", "id": "1".repeat(ID_MAX_LEN + 1) })),
    ];

    for body in bad {
        assert!(
            matches!(
                conn(&bytes(body.clone())),
                Err(ClientApiError::BadRequest(_))
            ),
            "{body}"
        );
    }
    let no_conn_id = json!({ "id": "1", "uuid": "u", "session_id": 1, "action": "close" });
    assert!(conn(&bytes(no_conn_id)).is_err());
    assert!(conn(b"not json").is_err());
}

fn file_body(extra: Value) -> Value {
    let info = json!({ "ip": "10.0.0.1", "name": "bob", "num": 12,
                       "files": [["a", 5], ["", 3]] });
    let mut body = json!({ "id": "123456789", "uuid": "dXVpZA==", "peer_id": "987",
                           "conn_id": 7, "type": 1, "path": "/tmp", "is_file": false,
                           "info": info.to_string(), "nonce": "n" });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    body
}

#[test]
fn parses_a_file_record_and_its_info_string() {
    let (_, report) = file(&bytes(file_body(json!({})))).unwrap();
    let (_, bare) = file(&bytes(file_body(json!({ "info": "{}" })))).unwrap();

    assert_eq!(report.direction, Direction::Receive);
    assert_eq!(report.info.file_count, 12);
    assert_eq!(
        report.info.files,
        vec![("a".to_owned(), 5), (String::new(), 3)]
    );
    assert_eq!(report.info.ip, Some("10.0.0.1".parse().unwrap()));
    assert_eq!(bare.info, FileInfo::default());
}

/// A newer client may append elements to the pairs; they are ignored, not fatal.
#[test]
fn longer_tuples_from_newer_clients_are_accepted() {
    let info = json!({ "files": [["a", 5, "x", { "y": 1 }]] }).to_string();
    let (_, report) = file(&bytes(file_body(json!({ "info": info })))).unwrap();
    let (_, auth) = record(json!({ "peer": ["987", "bob", "extra"], "type": 0 }));

    assert_eq!(report.info.files, vec![("a".to_owned(), 5)]);
    let ConnEvent::Authorized(a) = auth.event else {
        panic!()
    };
    assert_eq!((a.peer_id.as_str(), a.peer_name.as_str()), ("987", "bob"));
}

#[test]
fn rejects_unusable_file_records() {
    for extra in [
        json!({ "type": 2 }),
        json!({ "info": "not json" }),
        json!({ "info": { "num": 1 } }),
        json!({ "info": "{\"files\": [[\"a\"]]}" }),
        json!({ "is_file": null }),
        json!({ "path": null }),
        json!({ "peer_id": "1".repeat(ID_MAX_LEN + 1) }),
    ] {
        let body = file_body(extra.clone());
        assert!(file(&bytes(body)).is_err(), "{extra}");
    }
}

#[test]
fn parses_an_alarm_and_requires_an_object_info() {
    let body = |info: &str| {
        json!({ "id": "123456789", "uuid": "dXVpZA==", "typ": 9, "conn_id": 7,
                "info": info, "nonce": "n" })
    };

    let (_, report) = alarm(&bytes(body("{\"ip\":\"10.0.0.1\"}"))).unwrap();

    assert_eq!((report.typ, report.conn_id), (9, 7));
    assert_eq!(report.info.get("ip"), Some(&json!("10.0.0.1")));
    assert!(alarm(&bytes(body("[1]"))).is_err());
    assert!(alarm(&bytes(body("garbage"))).is_err());
}
