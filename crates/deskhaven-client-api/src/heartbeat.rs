//! `POST /api/heartbeat` (03 §2.1; `src/hbbs_http/sync.rs:232-277`): every 15 s, every 3 s
//! with live connections. No Authorization: the device is identified by `id` + `uuid` (06
//! §2); the work is `DeviceService::heartbeat` (no database access on the steady path).
//!
//! Request: `{id, uuid, ver, conns?, modified_at}`; `ver` is ignored, a missing `conns` means
//! none, a missing `modified_at` counts as 0.
//!
//! | case | status | body |
//! |---|---|---|
//! | verified device | 200 | object with the keys that apply, often `{}` (below) |
//! | unknown id, or uuid mismatch | 404 | `{"error": "ID_NOT_FOUND"}` (identical for both) |
//! | malformed body, missing / oversized `id` or `uuid` | 400 | `{"error": ...}` |
//! | body over [`MAX_HEARTBEAT_BODY`] | 413 | stack renderer |
//! | over the per-IP quota, or within 2 s of the device's previous heartbeat | 429 + `Retry-After` | `{"error": "Please try later"}` |
//!
//! Reply keys (each only when it applies): `sysinfo: true` (server lacks the device's
//! sysinfo), `disconnect: [ids]` (queued and still live), `strategy: {config_options}` and
//! `modified_at` (the client's version differs; `strategy` is left out when it would be
//! empty). A 4xx reply has none of these keys, so the client simply tries again next time.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use deskhaven_core::devices::{
    HEARTBEAT_MIN_INTERVAL, Heartbeat, HeartbeatOutcome, HeartbeatReply,
};
use deskhaven_core::state::AppState;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::ClientApiError;
use crate::client::Client;

/// Largest accepted body (bytes): a real heartbeat is about 100 bytes plus 12 per live
/// connection.
pub const MAX_HEARTBEAT_BODY: usize = 8 * 1024;
/// Longest accepted `id` (bytes): hbbs `MAX_PEER_ID_LEN`.
const ID_MAX_LEN: usize = 253;
/// Longest accepted `uuid` (base64 characters); machine ids are far shorter.
const UUID_MAX_LEN: usize = 512;

/// Error text of an unknown id or a uuid mismatch (the sysinfo endpoint's word for it).
pub const ID_NOT_FOUND: &str = "ID_NOT_FOUND";
const INVALID_BODY: &str = "Invalid request body";

/// The body as `sync.rs:236-244` builds it; unknown keys (`ver`) are ignored.
#[derive(Deserialize)]
struct Body {
    id: Option<String>,
    uuid: Option<String>,
    conns: Option<Vec<i32>>,
    modified_at: Option<i64>,
}

pub async fn heartbeat(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    body: Bytes,
) -> Result<Json<Value>, ClientApiError> {
    state
        .devices
        .admit_heartbeat(client)
        .map_err(|retry_after| ClientApiError::TooManyRequests { retry_after })?;
    let beat = parse(&body, client.addr)?;
    let outcome = state
        .devices
        .heartbeat(beat)
        .await
        .map_err(ClientApiError::internal)?;
    match outcome {
        HeartbeatOutcome::Reply(reply) => Ok(Json(render(reply))),
        HeartbeatOutcome::NotFound => Err(ClientApiError::NotFound(ID_NOT_FOUND.into())),
        HeartbeatOutcome::TooFrequent => Err(ClientApiError::TooManyRequests {
            retry_after: HEARTBEAT_MIN_INTERVAL,
        }),
    }
}

fn parse(raw: &[u8], ip: std::net::IpAddr) -> Result<Heartbeat, ClientApiError> {
    let body: Body = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let rustdesk_id = body.id.filter(|id| bounded(id, ID_MAX_LEN));
    let uuid = body.uuid.filter(|uuid| bounded(uuid, UUID_MAX_LEN));
    let (Some(rustdesk_id), Some(uuid)) = (rustdesk_id, uuid) else {
        return Err(invalid());
    };
    Ok(Heartbeat {
        rustdesk_id,
        uuid,
        ip,
        conns: body.conns.unwrap_or_default(),
        modified_at: body.modified_at.unwrap_or_default(),
    })
}

/// The reply object: only the keys that apply (the client acts on each present key).
fn render(reply: HeartbeatReply) -> Value {
    let mut map = Map::new();
    if reply.sysinfo {
        map.insert("sysinfo".to_owned(), Value::Bool(true));
    }
    if !reply.disconnect.is_empty() {
        map.insert("disconnect".to_owned(), json!(reply.disconnect));
    }
    if let Some(push) = reply.strategy {
        if !push.config_options.is_empty() {
            let strategy = json!({ "config_options": push.config_options });
            map.insert("strategy".to_owned(), strategy);
        }
        map.insert("modified_at".to_owned(), json!(push.modified_at));
    }
    Value::Object(map)
}

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_BODY.into())
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use deskhaven_core::devices::StrategyPush;
    use deskhaven_core::strategies::Options;

    use super::*;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn parse_json(value: Value) -> Result<Heartbeat, ClientApiError> {
        parse(&serde_json::to_vec(&value).unwrap(), IP)
    }

    #[test]
    fn parses_the_client_body_with_defaults_for_absent_keys() {
        let full = parse_json(json!({
            "id": "123456789", "uuid": "dXVpZA==", "ver": 1_004_001,
            "conns": [12, 13], "modified_at": 1_730_000_000_000_i64, "unknown": true
        }))
        .unwrap();
        let idle = parse_json(json!({ "id": "123456789", "uuid": "dXVpZA==" })).unwrap();

        assert_eq!(full.conns, vec![12, 13]);
        assert_eq!(full.modified_at, 1_730_000_000_000);
        assert_eq!((full.rustdesk_id.as_str(), full.ip), ("123456789", IP));
        assert_eq!((idle.conns, idle.modified_at), (Vec::new(), 0));
    }

    #[test]
    fn rejects_bodies_without_a_usable_id_or_uuid_or_with_mistyped_keys() {
        let bad = [
            json!([]),
            json!({ "uuid": "dXVpZA==" }),
            json!({ "id": "123456789" }),
            json!({ "id": "", "uuid": "dXVpZA==" }),
            json!({ "id": "1".repeat(ID_MAX_LEN + 1), "uuid": "dXVpZA==" }),
            json!({ "id": "123456789", "uuid": "a".repeat(UUID_MAX_LEN + 1) }),
            json!({ "id": "123456789", "uuid": "dXVpZA==", "conns": ["12"] }),
            json!({ "id": "123456789", "uuid": "dXVpZA==", "modified_at": "1" }),
        ];

        for body in bad {
            assert!(
                matches!(parse_json(body.clone()), Err(ClientApiError::BadRequest(_))),
                "{body}"
            );
        }
    }

    #[test]
    fn renders_only_the_keys_that_apply_and_keeps_empty_values() {
        let options: Options = [
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), String::new()),
        ]
        .into_iter()
        .collect();
        let full = HeartbeatReply {
            sysinfo: true,
            disconnect: vec![12],
            strategy: Some(StrategyPush {
                config_options: options,
                modified_at: 7,
            }),
        };
        let version_only = HeartbeatReply {
            strategy: Some(StrategyPush {
                config_options: Options::new(),
                modified_at: 8,
            }),
            ..HeartbeatReply::default()
        };

        assert_eq!(
            render(full),
            json!({
                "sysinfo": true, "disconnect": [12],
                "strategy": { "config_options": { "a": "1", "b": "" } }, "modified_at": 7
            })
        );
        assert_eq!(render(version_only), json!({ "modified_at": 8 }));
        assert_eq!(render(HeartbeatReply::default()), json!({}));
    }
}
