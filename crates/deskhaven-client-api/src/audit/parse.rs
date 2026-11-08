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

/// `post_conn_audit` (`connection.rs:1559-1572`) plus the note of `send_note`.
#[derive(Deserialize)]
struct ConnBody {
    id: Option<String>,
    uuid: Option<String>,
    nonce: Option<String>,
    conn_id: Option<i64>,
    session_id: Option<Value>,
    action: Option<String>,
    ip: Option<String>,
    /// `[my_id, my_name]`; a newer client may append elements.
    peer: Option<Vec<Value>>,
    #[serde(rename = "type")]
    conn_type: Option<i32>,
    primary_auth: Option<i32>,
    two_factor: Option<i32>,
    note: Option<String>,
}

/// `post_file_audit` (`connection.rs:1590-1625`).
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
    /// `[[name, size], ...]`; a newer client may append elements to a pair.
    files: Option<Vec<Vec<Value>>>,
}

/// `post_alarm_audit` (`connection.rs:1627-1656`).
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
        (None, Some(peer)) => {
            let (peer_id, peer_name) = pair(peer)?;
            Ok(ConnEvent::Authorized(Authorization {
                peer_id,
                peer_name,
                conn_type: required(body.conn_type)?,
                primary_auth: body.primary_auth,
                two_factor: body.two_factor,
            }))
        }
        _ => Err(invalid()),
    }
}

/// The first two elements of a client tuple (`[a, b, ...]`); elements a newer client appends
/// are ignored.
fn pair<A: DeserializeOwned, B: DeserializeOwned>(
    items: &[Value],
) -> Result<(A, B), ClientApiError> {
    let [a, b, ..] = items else {
        return Err(invalid());
    };
    let a = serde_json::from_value(a.clone()).map_err(|_| invalid())?;
    let b = serde_json::from_value(b.clone()).map_err(|_| invalid())?;
    Ok((a, b))
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
    let files = files
        .iter()
        .map(|file| pair(file))
        .collect::<Result<Vec<(String, i64)>, _>>()?;
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
mod tests;
