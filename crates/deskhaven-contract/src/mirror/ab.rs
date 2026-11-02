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

/// `POST /api/ab/personal` (03 §5.1).
///
/// `flutter/lib/models/ab_model.dart:262-292`: a body with an `error` key is an error
/// (registered as `HbbData<AbPersonal>`); `json['guid']` lands in a `String?`, and a
/// missing/null guid silently switches the client to legacy mode, so the mirror requires a
/// string.
#[derive(Debug, Deserialize)]
pub struct AbPersonal {
    pub guid: String,
}

/// `POST /api/ab/settings` (03 §5.1).
///
/// `flutter/lib/models/ab_model.dart:230-258`: `json['max_peer_one_ab'] ?? 0` lands in an
/// `int` (a non-integer throws); `0` means "no limit known". The mirror requires the key:
/// deskhaven always has a limit.
#[derive(Debug, Deserialize)]
pub struct AbSettings {
    pub max_peer_one_ab: i64,
}

/// A shared address book profile (03 §5.1).
///
/// `flutter/lib/common/hbbs/hbbs.dart:258-275`: `guid`, `name`, `owner`, `note` are
/// `String` / `String?` with `?? ''` (another type throws); `info` is `dynamic`; `rule` is
/// an `int` with `?? 0`. Rules other than 1/2/3 make the book read-only in the client
/// (`ab_model.dart:1392-1408`); the mirror requires `guid`, `name` and a valid `rule`,
/// without which the profile is useless.
#[derive(Debug, Deserialize)]
pub struct AbProfile {
    pub guid: String,
    pub name: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub info: Option<Value>,
    pub rule: AbRule,
}

/// `ShareRule` (`flutter/lib/common/hbbs/hbbs.dart:210-216`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbRule {
    Read,
    ReadWrite,
    FullControl,
}

impl<'de> Deserialize<'de> for AbRule {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        match i64::deserialize(de)? {
            1 => Ok(Self::Read),
            2 => Ok(Self::ReadWrite),
            3 => Ok(Self::FullControl),
            other => Err(serde::de::Error::custom(format!("unknown rule {other}"))),
        }
    }
}

/// An address book tag; `POST /api/ab/tags/<guid>` answers a JSON list of them.
///
/// `flutter/lib/models/ab_model.dart:1499-1531` (`_jsonDecodeRespList`, so the body must
/// be a list) and `flutter/lib/common/hbbs/hbbs.dart:277-286`: `name ?? ''` into a
/// `String`, `color ?? ''` into an `int`, so a missing color throws: required.
#[derive(Debug, Deserialize)]
pub struct AbTag {
    #[serde(default)]
    pub name: Option<String>,
    pub color: i64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn profiles_tags_and_settings_parse_as_the_client_reads_them() {
        let page: PagedResponse<AbProfile> = serde_json::from_value(json!({
            "total": 1,
            "data": [{ "guid": "g", "name": "team", "owner": "", "note": "", "rule": 2 }]
        }))
        .unwrap();
        let tags: Vec<AbTag> =
            serde_json::from_value(json!([{ "name": "a", "color": 4_294_901_760_i64 }])).unwrap();

        assert_eq!(page.data[0].rule, AbRule::ReadWrite);
        assert_eq!(tags[0].color, 0xFFFF_0000);
        assert!(serde_json::from_value::<AbTag>(json!({ "name": "a" })).is_err());
        assert!(
            serde_json::from_value::<AbProfile>(json!({ "guid": "g", "name": "n", "rule": 4 }))
                .is_err()
        );
        assert!(serde_json::from_value::<AbSettings>(json!({ "max_peer_one_ab": "1" })).is_err());
        assert!(serde_json::from_value::<AbPersonal>(json!({ "guid": null })).is_err());
    }

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
