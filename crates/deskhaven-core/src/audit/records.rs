//! File transfer (03 §2.5) and alarm (03 §2.6) records: one row each, as reported.

use std::net::IpAddr;

use deskhaven_db::entity::{alarm, file_transfer};
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::{DatabaseTransaction, DbErr, EntityTrait};
use serde_json::{Map, Value};

use crate::text;

pub use deskhaven_db::entity::file_transfer::Direction;

/// Files listed per record at most (the client sends the ten largest,
/// `src/server/connection.rs:1606-1608`).
pub const MAX_FILES: usize = 10;
const PEER_ID_MAX_CHARS: usize = 253;
const PATH_MAX_CHARS: usize = 4096;
const FILE_NAME_MAX_CHARS: usize = 1024;
/// Alarm `info` keys stored at most (the client sends up to five).
pub const MAX_INFO_KEYS: usize = 16;
/// Longest stored alarm `info` string, in characters.
pub const MAX_INFO_TEXT_CHARS: usize = 256;
const MAX_INFO_KEY_CHARS: usize = 64;
/// The keys of the client's alarm `info` objects (`src/server/connection.rs:1443-1478`,
/// `1662-1671`, `4303-4590`), kept before any other.
const KNOWN_INFO_KEYS: &[&str] = &["ip", "id", "name", "conn_type", "message"];

/// A file transfer the controlled device reports (`src/server/connection.rs:1594-1629`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileReport {
    /// The controller's RustDesk id.
    pub peer_id: String,
    pub conn_id: i64,
    /// `Send`: the device sent files to the controller (type 0, RemoteSend).
    pub direction: Direction,
    pub path: String,
    pub is_file: bool,
    pub info: FileInfo,
}

/// The `info` JSON string of a file record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInfo {
    /// The controller's address.
    pub ip: Option<IpAddr>,
    /// Files transferred (`num`), which may exceed the listed ones.
    pub file_count: i32,
    /// `[name, size]`, largest first; only the first [`MAX_FILES`] are stored.
    pub files: Vec<(String, i64)>,
}

/// A security alarm the controlled device reports (`src/server/connection.rs:1631-1660`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlarmReport {
    /// `AlarmAuditType` (`src/server/connection.rs:6546-6558`); unknown values are stored.
    pub typ: i32,
    pub conn_id: i64,
    /// The decoded `info` JSON string; stored flat and cleaned (`clean_info`).
    pub info: Map<String, Value>,
}

pub(super) async fn insert_file(
    tx: &DatabaseTransaction,
    device: &str,
    report: &FileReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    let info = &report.info;
    let row = file_transfer::ActiveModel {
        device_rustdesk_id: Set(device.to_owned()),
        peer_rustdesk_id: Set(text::clean(&report.peer_id, PEER_ID_MAX_CHARS, false)),
        conn_id: Set(report.conn_id),
        direction: Set(report.direction),
        path: Set(text::clean(&report.path, PATH_MAX_CHARS, false)),
        is_file: Set(report.is_file),
        file_count: Set(info.file_count.max(0)),
        files: Set(files_json(&info.files)),
        ip: Set(info.ip.map(|ip| ip.to_string())),
        created_at: Set(now),
        ..Default::default()
    };
    file_transfer::Entity::insert(row)
        .exec_without_returning(tx)
        .await
        .map(drop)
}

pub(super) async fn insert_alarm(
    tx: &DatabaseTransaction,
    device: &str,
    report: &AlarmReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    let row = alarm::ActiveModel {
        device_rustdesk_id: Set(device.to_owned()),
        typ: Set(report.typ),
        info: Set(Value::Object(clean_info(&report.info)).to_string()),
        conn_id: Set(report.conn_id),
        created_at: Set(now),
        ..Default::default()
    };
    alarm::Entity::insert(row)
        .exec_without_returning(tx)
        .await
        .map(drop)
}

/// Alarm `info` as stored: a flat object, the keys every client call site uses first
/// ([`KNOWN_INFO_KEYS`]), then the others in order, at most [`MAX_INFO_KEYS`]; keys and
/// strings cleaned and cut, numbers and booleans kept, other values (nested, null) dropped.
fn clean_info(info: &Map<String, Value>) -> Map<String, Value> {
    let known = KNOWN_INFO_KEYS
        .iter()
        .filter_map(|k| info.get_key_value(*k));
    let others = info
        .iter()
        .filter(|(k, _)| !KNOWN_INFO_KEYS.contains(&k.as_str()));
    known
        .chain(others)
        .filter_map(|(key, value)| {
            let key = text::clean(key, MAX_INFO_KEY_CHARS, false);
            (!key.is_empty()).then_some(key).zip(scalar(value))
        })
        .take(MAX_INFO_KEYS)
        .collect()
}

fn scalar(value: &Value) -> Option<Value> {
    match value {
        Value::String(s) => Some(Value::String(text::clean(s, MAX_INFO_TEXT_CHARS, false))),
        Value::Number(_) | Value::Bool(_) => Some(value.clone()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// `[[name, size], ...]` of the first [`MAX_FILES`] files, names cleaned.
fn files_json(files: &[(String, i64)]) -> String {
    let pairs = files.iter().take(MAX_FILES).map(|(name, size)| {
        let name = text::clean(name, FILE_NAME_MAX_CHARS, false);
        Value::Array(vec![Value::String(name), Value::from(*size)])
    });
    Value::Array(pairs.collect()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_capped_and_cleaned() {
        let files: Vec<(String, i64)> = (0..12).map(|i| (format!("f{i}\u{202E}"), i)).collect();

        let json: Value = serde_json::from_str(&files_json(&files)).unwrap();

        let listed = json.as_array().unwrap();
        assert_eq!(listed.len(), MAX_FILES);
        assert_eq!(listed[0], serde_json::json!(["f0", 0]));
    }
}
