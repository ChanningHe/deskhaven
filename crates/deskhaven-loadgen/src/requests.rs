//! The tests hold these bodies against the contract fixtures.

use serde_json::{Map, Value, json};

use crate::plan::{AuditCall, AuditStep, HeartbeatReply, SysinfoOutcome};
use crate::stats::Failure;

pub const CLIENT_VERSION: &str = "1.4.1";
/// The controller every simulated connection comes from (TEST-NET-2, RFC 5737).
const CONTROLLER_IP: &str = "198.51.100.4";
const CONTROLLER_ID: &str = "987654321";
const CONTROLLER_NAME: &str = "loadgen@controller";
/// `type` 0: remote control; `primary_auth` 3: permanent password.
const CONN_TYPE_REMOTE: u8 = 0;
const AUTH_PERMANENT_PASSWORD: u8 = 3;
const OK: u16 = 200;
const EXCERPT: usize = 40;

#[derive(Clone, Copy, Debug)]
pub struct Sender<'a> {
    pub id: &'a str,
    pub uuid_b64: &'a str,
}

/// `conns` only when there are live connections.
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

/// The client's `get_sysinfo` plus `version`, `id`, `uuid`, without presets.
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

/// From the controlled side, with a fresh `nonce`.
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

/// 200 with a JSON object.
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

/// Plain text `SYSINFO_UPDATED` is success; anything else is recorded as a failure.
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

/// Only 2xx with an empty body is stored.
pub fn audit_reply(status: u16, body: &[u8]) -> Result<(), Failure> {
    match (status, body.is_empty()) {
        (200..=299, true) => Ok(()),
        (200..=299, false) => Err(Failure::Body(excerpt(body))),
        _ => Err(Failure::Status(status)),
    }
}

fn excerpt(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(EXCERPT)]).into_owned()
}

#[cfg(test)]
mod tests;
