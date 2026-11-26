use std::collections::HashMap;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::AbPeer;

/// `data` goes through `jsonDecode`, so it must be a *string*; `null` is an empty book. The
/// client ignores a non-integer `licensed_devices`; the mirror requires an integer when present.
#[derive(Debug)]
pub struct LegacyAbReply {
    pub data: LegacyAbData,
    pub licensed_devices: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Deserialize)]
struct Envelope {
    data: String,
    #[serde(default)]
    licensed_devices: Option<i64>,
    #[serde(default)]
    updated_at: Option<i64>,
}

impl<'de> Deserialize<'de> for LegacyAbReply {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(de)?;
        if map.contains_key("error") {
            return Err(D::Error::custom(
                "a top-level \"error\" key is an error reply",
            ));
        }
        let envelope: Envelope =
            serde_json::from_value(Value::Object(map)).map_err(D::Error::custom)?;
        let data = serde_json::from_str(&envelope.data)
            .map_err(|err| D::Error::custom(format!("data is not the legacy document: {err}")))?;
        Ok(Self {
            data,
            licensed_devices: envelope.licensed_devices,
            updated_at: envelope.updated_at,
        })
    }
}

/// `tag_colors` is read only when a *string* (an object is silently ignored, a missing key makes
/// the client invent colours); the mirror requires it because deskhaven always sends one.
#[derive(Debug, Default)]
pub struct LegacyAbData {
    pub tags: Vec<String>,
    pub peers: Vec<AbPeer>,
    pub tag_colors: HashMap<String, i64>,
}

impl<'de> Deserialize<'de> for LegacyAbData {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            tags: Vec<String>,
            #[serde(default)]
            peers: Vec<AbPeer>,
            tag_colors: String,
        }
        let raw = Raw::deserialize(de)?;
        let tag_colors = serde_json::from_str(&raw.tag_colors).map_err(|err| {
            D::Error::custom(format!("tag_colors is not a name to int map: {err}"))
        })?;
        Ok(Self {
            tags: raw.tags,
            peers: raw.peers,
            tag_colors,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reply(data: &Value) -> Value {
        json!({ "data": data.to_string(), "licensed_devices": 0 })
    }

    #[test]
    fn the_document_parses_as_the_client_reads_it() {
        let data = json!({
            "tags": ["a"],
            "peers": [{ "id": "1", "hash": "h", "tags": ["a"] }],
            "tag_colors": json!({ "a": 4_278_190_335_i64 }).to_string()
        });

        let parsed: LegacyAbReply = serde_json::from_value(reply(&data)).unwrap();

        assert_eq!(parsed.data.tags, ["a"]);
        assert_eq!(parsed.data.peers[0].hash.as_deref(), Some("h"));
        assert_eq!(parsed.data.tag_colors["a"], 0xFF00_00FF);
        assert_eq!(parsed.licensed_devices, Some(0));
    }

    #[test]
    fn what_the_client_would_misread_is_refused() {
        let colors = json!({ "a": 1 }).to_string();
        let bad = [
            json!({ "error": "x" }),
            json!({ "data": { "tags": [] } }),
            json!({ "data": "not json" }),
            reply(&json!({ "tags": [1], "tag_colors": "{}" })),
            reply(&json!({ "tags": [], "tag_colors": { "a": 1 } })),
            reply(&json!({ "tags": [], "tag_colors": "{\"a\": \"red\"}" })),
            reply(&json!({ "tags": [], "tag_colors": colors, "peers": {} })),
            json!({ "data": "{\"tags\":[],\"tag_colors\":\"{}\"}", "licensed_devices": "5" }),
        ];

        for body in bad {
            assert!(
                serde_json::from_value::<LegacyAbReply>(body.clone()).is_err(),
                "{body}"
            );
        }
    }
}
