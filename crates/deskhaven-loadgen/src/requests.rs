//! Bodies of the simulated client API calls and how the client reads the answers (03 §2,
//! client `23e5bb4`). The tests hold them against the contract fixtures.

use serde_json::{Map, Value, json};

use crate::plan::{AuditCall, AuditStep, HeartbeatReply, SysinfoOutcome};
use crate::stats::Failure;

/// The client version the simulated devices report.
pub const CLIENT_VERSION: &str = "1.4.1";
/// The controller every simulated connection comes from (TEST-NET-2, RFC 5737).
const CONTROLLER_IP: &str = "198.51.100.4";
const CONTROLLER_ID: &str = "987654321";
const CONTROLLER_NAME: &str = "loadgen@controller";
/// `type` 0: remote control; `primary_auth` 3: permanent password (03 §2.3).
const CONN_TYPE_REMOTE: u8 = 0;
const AUTH_PERMANENT_PASSWORD: u8 = 3;
const OK: u16 = 200;
/// Characters of an unexpected body kept in the failure table.
const EXCERPT: usize = 40;

/// Who sends: the id and base64 uuid every device endpoint carries.
#[derive(Clone, Copy, Debug)]
pub struct Sender<'a> {
    pub id: &'a str,
    pub uuid_b64: &'a str,
}

/// `POST /api/heartbeat` (`sync.rs:236-245`): `conns` only when there are live connections.
pub fn heartbeat(from: Sender<'_>, conns: &[i32], modified_at: i64) -> Value {
    let mut body = json!({
        "id": from.id,
        "uuid": from.uuid_b64,
        "ver": hbb_common::get_version_number(CLIENT_VERSION),
        "modified_at": modified_at,
    });
    if !conns.is_empty() {
        body["conns"] = json!(conns);
    }
    body
}

/// `POST /api/sysinfo` (`sync.rs:126-136`, `common.rs:962-1006`): `get_sysinfo` plus
/// `version`, `id`, `uuid`, without presets.
pub fn sysinfo(from: Sender<'_>, index: usize) -> Value {
    json!({
        "cpu": "AMD Ryzen 5 5600G with Radeon Graphics, 3.89GHz, 12/6 cores",
        "memory": "15.5GB",
        "os": "ubuntu / Linux 24.04 Ubuntu",
        "hostname": format!("loadgen-{index}"),
        "username": "user",
        "version": CLIENT_VERSION,
        "id": from.id,
        "uuid": from.uuid_b64,
    })
}

/// `POST /api/audit/conn` from the controlled side (03 §2.3), with a fresh `nonce`.
pub fn audit(from: Sender<'_>, call: &AuditCall, nonce: &str) -> Value {
    let mut body = match call.step {
        AuditStep::New => json!({ "ip": CONTROLLER_IP, "action": "new" }),
        AuditStep::Authorized => json!({
            "peer": [CONTROLLER_ID, CONTROLLER_NAME],
            "type": CONN_TYPE_REMOTE,
            "primary_auth": AUTH_PERMANENT_PASSWORD,
        }),
        AuditStep::Close => json!({ "action": "close" }),
    };
    body["id"] = json!(from.id);
    body["uuid"] = json!(from.uuid_b64);
    body["conn_id"] = json!(call.conn_id);
    body["session_id"] = json!(call.session_id);
    body["nonce"] = json!(nonce);
    body
}

/// A heartbeat answer: 200 with a JSON object (`sync.rs:248-277`).
pub fn heartbeat_reply(status: u16, body: &[u8]) -> Result<HeartbeatReply, Failure> {
    if status != OK {
        return Err(Failure::Status(status));
    }
    let map: Map<String, Value> =
        serde_json::from_slice(body).map_err(|_| Failure::Body("not a JSON object".to_owned()))?;
    Ok(HeartbeatReply {
        // `rsp.remove("sysinfo").is_some()`: presence counts, whatever the value.
        sysinfo: map.contains_key("sysinfo"),
        modified_at: map.get("modified_at").and_then(Value::as_i64),
    })
}

/// A sysinfo answer as the client classifies it (`sync.rs:211-228`): plain text
/// `SYSINFO_UPDATED` is success, anything else is recorded as a failure.
pub fn sysinfo_reply(status: u16, body: &[u8]) -> (SysinfoOutcome, Result<(), Failure>) {
    if status != OK {
        return (SysinfoOutcome::Other, Err(Failure::Status(status)));
    }
    match body {
        b"SYSINFO_UPDATED" => (SysinfoOutcome::Updated, Ok(())),
        b"ID_NOT_FOUND" => (
            SysinfoOutcome::IdNotFound,
            Err(Failure::Body("ID_NOT_FOUND".to_owned())),
        ),
        other => (SysinfoOutcome::Other, Err(Failure::Body(excerpt(other)))),
    }
}

/// An audit answer: only 2xx with an empty body is stored (03 §1.3, §2.3).
pub fn audit_reply(status: u16, body: &[u8]) -> Result<(), Failure> {
    match (status, body.is_empty()) {
        (200..=299, true) => Ok(()),
        (200..=299, false) => Err(Failure::Body(excerpt(body))),
        _ => Err(Failure::Status(status)),
    }
}

/// The start of an unexpected body.
fn excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(EXCERPT)]).into_owned()
}

#[cfg(test)]
mod tests;
