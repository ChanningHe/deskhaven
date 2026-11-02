//! Device endpoints (03 §2).

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// `strategy` of a heartbeat response.
///
/// `src/hbbs_http/sync.rs:45-51`: both maps default to empty; values are
/// strings, an empty one deletes the option (03 §2.1).
#[derive(Debug, Deserialize)]
pub struct StrategyOptions {
    #[serde(default)]
    pub config_options: HashMap<String, String>,
    #[serde(default)]
    pub extra: HashMap<String, String>,
}

/// `POST /api/heartbeat` response (03 §2.1).
///
/// `src/hbbs_http/sync.rs:250-277`: the body is a JSON object; every key is
/// optional. The client *ignores* a key whose value has the wrong type
/// (`if let Ok(..)`); the mirror rejects it instead, since an ignored key
/// is a lost disconnect, strategy or timestamp.
#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    /// Presence is the signal (`rsp.remove("sysinfo").is_some()`), so even
    /// `"sysinfo": null` is `Some`.
    #[serde(default, deserialize_with = "present")]
    pub sysinfo: Option<Value>,
    #[serde(default)]
    pub disconnect: Option<Vec<i32>>,
    #[serde(default)]
    pub strategy: Option<StrategyOptions>,
    #[serde(default)]
    pub modified_at: Option<i64>,
}

/// `Some` for any present value, `null` included.
fn present<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(de).map(Some)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> Result<HeartbeatResponse, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn contract_example_parses() {
        let rsp = parse(json!({
            "sysinfo": true,
            "disconnect": [12],
            "strategy": { "config_options": { "enable-file-transfer": "N", "allow-remote-config-modification": "" }, "extra": {} },
            "modified_at": 1_730_000_100
        }))
        .unwrap();

        assert_eq!(rsp.disconnect, Some(vec![12]));
        assert_eq!(rsp.modified_at, Some(1_730_000_100));
    }

    #[test]
    fn sysinfo_presence_counts_even_when_null() {
        assert!(parse(json!({ "sysinfo": null })).unwrap().sysinfo.is_some());
        assert!(parse(json!({})).unwrap().sysinfo.is_none());
    }

    #[test]
    fn mistyped_keys_are_rejected() {
        assert!(parse(json!({ "disconnect": ["12"] })).is_err());
        assert!(parse(json!({ "modified_at": "1" })).is_err());
        assert!(parse(json!({ "strategy": { "config_options": { "a": true } } })).is_err());
    }
}
