//! Address book and group endpoints (03 §4, §5).

use serde::Deserialize;
use serde_json::Value;

/// Paged list: `{total, data: [...]}` (03 §1.3).
///
/// `flutter/lib/models/ab_model.dart:1457-1471` (also `:333-337` and the
/// group lists in `flutter/lib/models/group_model.dart`): nothing is read
/// unless `total` is present, and `total` lands in an `int`. A non-list
/// `data` is silently skipped by the client; the mirror requires a list
/// when the key is present.
#[derive(Debug, Deserialize)]
pub struct PagedResponse<T> {
    pub total: i64,
    #[serde(default = "Vec::new")]
    pub data: Vec<T>,
}

/// Address book peer (03 §5.1).
///
/// `flutter/lib/models/peer_model.dart:33-48`: every key defaults (`?? ''`),
/// but a present value of another type throws, except:
/// - `forceAlwaysRelay` is compared with the *string* `'true'`; a JSON
///   `true` would be read as `false`, so the mirror requires a string.
/// - `note` of another type is read as `''`; the mirror requires a string.
#[derive(Debug, Deserialize)]
pub struct AbPeer {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<Value>>,
    #[serde(default, rename = "forceAlwaysRelay")]
    pub force_always_relay: Option<String>,
    #[serde(default, rename = "rdpPort")]
    pub rdp_port: Option<String>,
    #[serde(default, rename = "rdpUsername")]
    pub rdp_username: Option<String>,
    #[serde(default, rename = "loginName")]
    pub login_name: Option<String>,
    #[serde(default)]
    pub device_group_name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub same_server: Option<bool>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The Peer example of 03 §5.1.
    fn peer() -> Value {
        json!({
            "id": "123", "hash": "", "password": "", "username": "", "hostname": "", "platform": "",
            "alias": "", "tags": ["a"], "forceAlwaysRelay": "false", "rdpPort": "", "rdpUsername": "",
            "loginName": "", "device_group_name": "", "note": "", "same_server": true
        })
    }

    #[test]
    fn contract_example_parses_in_a_page() {
        let page: PagedResponse<AbPeer> =
            serde_json::from_value(json!({ "total": 1, "data": [peer()] })).unwrap();

        assert_eq!(page.total, 1);
        assert_eq!(page.data[0].force_always_relay.as_deref(), Some("false"));
    }

    #[test]
    fn force_always_relay_must_be_a_string() {
        let mut relay = peer();
        relay["forceAlwaysRelay"] = json!(true);

        assert!(serde_json::from_value::<AbPeer>(relay).is_err());
    }

    #[test]
    fn page_requires_total() {
        let missing = serde_json::from_value::<PagedResponse<AbPeer>>(json!({ "data": [] }));
        let empty = serde_json::from_value::<PagedResponse<AbPeer>>(json!({ "total": 0 }));

        assert!(missing.is_err());
        assert!(empty.unwrap().data.is_empty());
    }
}
