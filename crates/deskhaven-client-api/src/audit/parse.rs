//! Audit request bodies, as `src/server/connection.rs` and `src/ui_session_interface.rs`
//! build them, into the core's records. Unknown keys (`conn_audit_ref`, P1) are ignored; a
//! present key of the wrong type, or a missing required one, is a 400.

use std::net::IpAddr;

use deskhaven_core::audit::{
    AlarmReport, Authorization, ConnEvent, ConnReport, ControllerNote, Direction, Envelope,
    FileInfo, FileReport, SessionId,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::ClientApiError;

/// Longest accepted `id` / `peer_id` (bytes): hbbs `MAX_PEER_ID_LEN`.
const ID_MAX_LEN: usize = 253;
/// Longest accepted `uuid` (base64 characters); machine ids are far shorter.
const UUID_MAX_LEN: usize = 512;
/// Longest accepted `nonce` (the client sends a 36-character UUID).
const NONCE_MAX_LEN: usize = 64;
const INVALID_BODY: &str = "Invalid request body";

/// What `/api/audit/conn` received.
#[derive(Debug)]
pub(super) enum ConnMessage {
    /// A record of the controlled device (03 §2.3).
    Record(Envelope, ConnReport),
    /// The controller's session note (03 §2.4): `note`, without `conn_id` and `action`.
    Note(ControllerNote),
}

/// `post_conn_audit` (`connection.rs:1563-1576`) plus the note of `send_note`.
#[derive(Deserialize)]
struct ConnBody {
    id: Option<String>,
    uuid: Option<String>,
    nonce: Option<String>,
    conn_id: Option<i64>,
    session_id: Option<Value>,
    action: Option<String>,
    ip: Option<String>,
    peer: Option<(String, String)>,
    #[serde(rename = "type")]
    conn_type: Option<i32>,
    primary_auth: Option<i32>,
    two_factor: Option<i32>,
    note: Option<String>,
}

/// `post_file_audit` (`connection.rs:1594-1629`).
#[derive(Deserialize)]
struct FileBody {
    id: Option<String>,
    uuid: Option<String>,
    nonce: Option<String>,
    peer_id: Option<String>,
    conn_id: Option<i64>,
    #[serde(rename = "type")]
    direction: Option<i32>,
    path: Option<String>,
    is_file: Option<bool>,
    info: Option<String>,
}

/// The `info` JSON string of a file record: `{ip, name, num, files: [[name, size]...]}`.
#[derive(Deserialize)]
struct FileInfoBody {
    ip: Option<String>,
    num: Option<u64>,
    files: Option<Vec<(String, i64)>>,
}

/// `post_alarm_audit` (`connection.rs:1631-1660`).
#[derive(Deserialize)]
struct AlarmBody {
    id: Option<String>,
    uuid: Option<String>,
    nonce: Option<String>,
    typ: Option<i32>,
    conn_id: Option<i64>,
    info: Option<String>,
}

pub(super) fn conn(raw: &[u8]) -> Result<ConnMessage, ClientApiError> {
    let body: ConnBody = json(raw)?;
    if body.note.is_some() && body.conn_id.is_none() && body.action.is_none() {
        return note(body);
    }
    let event = conn_event(&body)?;
    let report = ConnReport {
        conn_id: required(body.conn_id)?,
        session_id: session_id(body.session_id.as_ref())?,
        event,
    };
    let envelope = envelope(body.id, body.uuid, body.nonce)?;
    Ok(ConnMessage::Record(envelope, report))
}

/// `new` (with `ip`), `close`, or no action with `peer` (the authorization).
fn conn_event(body: &ConnBody) -> Result<ConnEvent, ClientApiError> {
    match (body.action.as_deref(), &body.peer) {
        (Some("new"), _) => Ok(ConnEvent::New {
            ip: body.ip.as_deref().map(ip).transpose()?,
        }),
        (Some("close"), _) => Ok(ConnEvent::Close),
        (None, Some((peer_id, peer_name))) => Ok(ConnEvent::Authorized(Authorization {
            peer_id: peer_id.clone(),
            peer_name: peer_name.clone(),
            conn_type: required(body.conn_type)?,
            primary_auth: body.primary_auth,
            two_factor: body.two_factor,
        })),
        _ => Err(invalid()),
    }
}

/// The note names a session: 0 (no session yet) names none.
fn note(body: ConnBody) -> Result<ConnMessage, ClientApiError> {
    let session_id = session_id(body.session_id.as_ref())?;
    let rustdesk_id = body.id.filter(|id| bounded(id, ID_MAX_LEN));
    let (Some(rustdesk_id), Some(note), false) = (rustdesk_id, body.note, session_id.is_unbound())
    else {
        return Err(invalid());
    };
    Ok(ConnMessage::Note(ControllerNote {
        rustdesk_id,
        session_id,
        note,
    }))
}

pub(super) fn file(raw: &[u8]) -> Result<(Envelope, FileReport), ClientApiError> {
    let body: FileBody = json(raw)?;
    let direction = match required(body.direction)? {
        0 => Direction::Send,
        1 => Direction::Receive,
        _ => return Err(invalid()),
    };
    let peer_id = required(body.peer_id.filter(|id| id.len() <= ID_MAX_LEN))?;
    let report = FileReport {
        peer_id,
        conn_id: required(body.conn_id)?,
        direction,
        path: required(body.path)?,
        is_file: required(body.is_file)?,
        info: file_info(&required(body.info)?)?,
    };
    Ok((envelope(body.id, body.uuid, body.nonce)?, report))
}

fn file_info(raw: &str) -> Result<FileInfo, ClientApiError> {
    let info: FileInfoBody = json(raw.as_bytes())?;
    let files = info.files.unwrap_or_default();
    let count = info.num.unwrap_or(files.len() as u64);
    Ok(FileInfo {
        ip: info.ip.as_deref().map(ip).transpose()?,
        file_count: i32::try_from(count).unwrap_or(i32::MAX),
        files,
    })
}

pub(super) fn alarm(raw: &[u8]) -> Result<(Envelope, AlarmReport), ClientApiError> {
    let body: AlarmBody = json(raw)?;
    let info: Map<String, Value> = json(required(body.info)?.as_bytes())?;
    let report = AlarmReport {
        typ: required(body.typ)?,
        conn_id: required(body.conn_id)?,
        info,
    };
    Ok((envelope(body.id, body.uuid, body.nonce)?, report))
}

fn envelope(
    id: Option<String>,
    uuid: Option<String>,
    nonce: Option<String>,
) -> Result<Envelope, ClientApiError> {
    let rustdesk_id = id.filter(|id| bounded(id, ID_MAX_LEN));
    let uuid = uuid.filter(|uuid| bounded(uuid, UUID_MAX_LEN));
    let (Some(rustdesk_id), Some(uuid)) = (rustdesk_id, uuid) else {
        return Err(invalid());
    };
    let nonce_ok =
        |n: &String| bounded(n, NONCE_MAX_LEN) && n.bytes().all(|b| b.is_ascii_graphic());
    if nonce.as_ref().is_some_and(|n| !nonce_ok(n)) {
        return Err(invalid());
    }
    Ok(Envelope {
        rustdesk_id,
        uuid,
        nonce,
    })
}

fn session_id(value: Option<&Value>) -> Result<SessionId, ClientApiError> {
    value.and_then(SessionId::from_json).ok_or_else(invalid)
}

fn ip(raw: &str) -> Result<IpAddr, ClientApiError> {
    raw.parse().map_err(|_| invalid())
}

fn json<T: DeserializeOwned>(raw: &[u8]) -> Result<T, ClientApiError> {
    serde_json::from_slice(raw).map_err(|_| invalid())
}

fn required<T>(value: Option<T>) -> Result<T, ClientApiError> {
    value.ok_or_else(invalid)
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_BODY.into())
}

#[cfg(test)]
mod tests {
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
}
