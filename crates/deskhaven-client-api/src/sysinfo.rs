//! `POST /api/sysinfo` (03 §2.2; `src/hbbs_http/sync.rs:126-228`): a device reports its
//! system information, with deployment presets on a custom client. No Authorization: the
//! device is identified by `id` + `uuid` (06 §2, `deskhaven_core::devices`).
//!
//! | case | status | body (plain text) |
//! |---|---|---|
//! | registered id, matching uuid (stored or unchanged) | 200 | `SYSINFO_UPDATED` |
//! | unknown id, or uuid mismatch | 200 | `ID_NOT_FOUND` (client retries next tick, 3 s) |
//! | malformed body, missing / oversized `id` or `uuid` | 400 | `{"error": ...}` (client retries in 120 s) |
//! | body over [`MAX_SYSINFO_BODY`] | 413 | stack renderer |
//! | over the per-IP quota, or a changing write within 30 s of the last (`deskhaven_core::devices` limits) | 429 + `Retry-After` | `{"error": "Please try later"}` (client retries in 120 s; never loops) |
//!
//! The client compares the body with the two texts exactly (`x == "SYSINFO_UPDATED"`), so no
//! trailing newline.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use deskhaven_core::devices::{
    AddressBookPreset, Presets, SYSINFO_WRITE_INTERVAL, Sysinfo, SysinfoOutcome, SysinfoReport,
};
use deskhaven_core::state::AppState;
use serde::Deserialize;

use crate::ClientApiError;
use crate::client::Client;

/// Largest accepted body (bytes): a real report is well under 1 KiB, presets included.
pub const MAX_SYSINFO_BODY: usize = 16 * 1024;
/// Longest accepted `id` (bytes): hbbs `MAX_PEER_ID_LEN`.
const ID_MAX_LEN: usize = 253;
/// Longest accepted `uuid` (base64 characters); machine ids are far shorter.
const UUID_MAX_LEN: usize = 512;

pub const UPDATED: &str = "SYSINFO_UPDATED";
pub const ID_NOT_FOUND: &str = "ID_NOT_FOUND";
const INVALID_BODY: &str = "Invalid request body";

/// The body as `sync.rs` builds it; unknown keys are ignored.
#[derive(Deserialize)]
struct Body {
    id: Option<String>,
    uuid: Option<String>,
    hostname: Option<String>,
    username: Option<String>,
    os: Option<String>,
    cpu: Option<String>,
    memory: Option<String>,
    version: Option<String>,
    #[serde(rename = "preset-user-name")]
    preset_user_name: Option<String>,
    #[serde(rename = "preset-device-group-name")]
    preset_device_group_name: Option<String>,
    #[serde(rename = "preset-strategy-name")]
    preset_strategy_name: Option<String>,
    #[serde(rename = "preset-note")]
    preset_note: Option<String>,
    #[serde(rename = "preset-address-book-name")]
    preset_address_book_name: Option<String>,
    #[serde(rename = "preset-address-book-tag")]
    preset_address_book_tag: Option<String>,
    #[serde(rename = "preset-address-book-alias")]
    preset_address_book_alias: Option<String>,
    #[serde(rename = "preset-address-book-password")]
    preset_address_book_password: Option<String>,
    #[serde(rename = "preset-address-book-note")]
    preset_address_book_note: Option<String>,
}

pub async fn sysinfo(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    body: Bytes,
) -> Result<&'static str, ClientApiError> {
    state
        .devices
        .admit_sysinfo(client)
        .map_err(|retry_after| ClientApiError::TooManyRequests { retry_after })?;
    let report = parse(&body)?;
    let outcome = state
        .devices
        .report_sysinfo(report)
        .await
        .map_err(ClientApiError::internal)?;
    match outcome {
        SysinfoOutcome::Stored | SysinfoOutcome::Unchanged => Ok(UPDATED),
        SysinfoOutcome::NotFound => Ok(ID_NOT_FOUND),
        SysinfoOutcome::TooFrequent => Err(ClientApiError::TooManyRequests {
            retry_after: SYSINFO_WRITE_INTERVAL,
        }),
    }
}

fn parse(raw: &[u8]) -> Result<SysinfoReport, ClientApiError> {
    let body: Body = serde_json::from_slice(raw).map_err(|_| invalid())?;
    let rustdesk_id = body.id.clone().filter(|id| bounded(id, ID_MAX_LEN));
    let uuid = body.uuid.clone().filter(|uuid| bounded(uuid, UUID_MAX_LEN));
    let (Some(rustdesk_id), Some(uuid)) = (rustdesk_id, uuid) else {
        return Err(invalid());
    };
    let sysinfo = Sysinfo {
        hostname: body.hostname.clone(),
        username: body.username.clone(),
        os: body.os.clone(),
        cpu: body.cpu.clone(),
        memory: body.memory.clone(),
        version: body.version.clone(),
    };
    Ok(SysinfoReport {
        rustdesk_id,
        uuid,
        sysinfo,
        presets: presets(body),
    })
}

/// The client sends only non-empty presets; an empty one counts as absent all the same. The
/// password moves into a zeroizing buffer without a copy.
fn presets(body: Body) -> Presets {
    let address_book = given(body.preset_address_book_name).map(|name| AddressBookPreset {
        name,
        tag: given(body.preset_address_book_tag),
        alias: given(body.preset_address_book_alias),
        password: given(body.preset_address_book_password).map(Into::into),
        note: given(body.preset_address_book_note),
    });
    Presets {
        user_name: given(body.preset_user_name),
        device_group_name: given(body.preset_device_group_name),
        strategy_name: given(body.preset_strategy_name),
        note: given(body.preset_note),
        address_book,
    }
}

fn given(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
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

    fn parse_json(value: serde_json::Value) -> Result<SysinfoReport, ClientApiError> {
        parse(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn parses_the_client_body_with_presets() {
        let report = parse_json(json!({
            "cpu": "8/4 cores", "memory": "15.5GB", "os": "linux", "hostname": "pc",
            "version": "1.4.1", "id": "123456789", "uuid": "dXVpZA==",
            "preset-user-name": "pat", "preset-device-group-name": "Office",
            "preset-strategy-name": "Locked", "preset-note": "n",
            "preset-address-book-name": "Team", "preset-address-book-tag": "t",
            "preset-address-book-alias": "a", "preset-address-book-password": "p",
            "preset-address-book-note": "abn", "unknown": 1
        }))
        .unwrap();

        assert_eq!(report.rustdesk_id, "123456789");
        assert_eq!(report.uuid, "dXVpZA==");
        assert_eq!(report.sysinfo.hostname.as_deref(), Some("pc"));
        assert_eq!(report.sysinfo.username, None);
        assert_eq!(report.presets.user_name.as_deref(), Some("pat"));
        assert_eq!(report.presets.device_group_name.as_deref(), Some("Office"));
        assert_eq!(report.presets.strategy_name.as_deref(), Some("Locked"));
        assert_eq!(report.presets.note.as_deref(), Some("n"));
        let book = report.presets.address_book.unwrap();
        assert_eq!(book.name, "Team");
        assert_eq!(book.tag.as_deref(), Some("t"));
        assert_eq!(book.alias.as_deref(), Some("a"));
        assert_eq!(book.password.as_deref().map(String::as_str), Some("p"));
        assert_eq!(book.note.as_deref(), Some("abn"));
    }

    #[test]
    fn empty_presets_are_absent_and_book_fields_need_a_book_name() {
        let report = parse_json(json!({
            "id": "123456789", "uuid": "dXVpZA==",
            "preset-user-name": "", "preset-address-book-tag": "t"
        }))
        .unwrap();

        assert_eq!(report.presets, Presets::default());
    }

    #[test]
    fn rejects_bodies_without_a_usable_id_or_uuid() {
        let bad = [
            json!([]),
            json!({ "uuid": "dXVpZA==" }),
            json!({ "id": "123456789" }),
            json!({ "id": "", "uuid": "dXVpZA==" }),
            json!({ "id": "1".repeat(ID_MAX_LEN + 1), "uuid": "dXVpZA==" }),
            json!({ "id": "123456789", "uuid": "a".repeat(UUID_MAX_LEN + 1) }),
            json!({ "id": 123456789, "uuid": "dXVpZA==" }),
            json!({ "id": "123456789", "uuid": "dXVpZA==", "hostname": 5 }),
        ];

        for body in bad {
            assert!(
                matches!(parse_json(body.clone()), Err(ClientApiError::BadRequest(_))),
                "{body}"
            );
        }
        assert!(parse(b"not json").is_err());
    }
}
