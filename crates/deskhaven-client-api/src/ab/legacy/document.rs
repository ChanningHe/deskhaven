//! The legacy address book document: the JSON *string* in the `data` field of
//! `GET /api/ab`, `POST /api/ab/get` and `POST /api/ab`.
//!
//! ```text
//! {"tags": ["a"],
//!  "peers": [{"id": "1", "username": "", "hostname": "", "platform": "", "alias": "",
//!             "tags": ["a"], "hash": "..."}],
//!  "tag_colors": "{\"a\": 4278190335}"}        <- a JSON string again, name -> ARGB int
//! ```
//!
//! Written by `_serialize` (`flutter/lib/models/ab_model.dart:1309-1323`), read by
//! `_deserialize` (`:1325-1356`) and, in the Sciter UI, by `JSON.parse` / `JSON.stringify`
//! (`src/ui/ab.tis:650-670`).
//!
//! Reading a peer reuses the per-item API's codec (`Peer::to_json`, `PeerPatch::from_json`),
//! so every field both APIs share is one definition; a legacy client ignores the fields it
//! does not know and a push without them leaves the stored values alone.

use deskhaven_core::addressbooks::{BookContents, BookReplacement, PeerPatch, ReplacementTag};
use serde_json::{Map, Value};

use crate::ClientApiError;
use crate::ab::ab_error;

const INVALID_DATA: &str = "Invalid data";

/// The document for `contents`, as the string clients put in `data`. Peers come ordered by
/// id and tags by name (what the book stores; a legacy client's own tag order is not kept).
pub fn render(contents: &BookContents) -> String {
    let peers: Vec<Value> = contents
        .peers
        .iter()
        .map(|peer| Value::Object(peer.to_json()))
        .collect();
    let tags: Vec<Value> = contents
        .tags
        .iter()
        .map(|tag| Value::String(tag.name.clone()))
        .collect();
    let colors: Map<String, Value> = contents
        .tags
        .iter()
        .map(|tag| (tag.name.clone(), Value::from(tag.color)))
        .collect();
    let document = Map::from_iter([
        ("tags".to_owned(), Value::Array(tags)),
        ("peers".to_owned(), Value::Array(peers)),
        (
            "tag_colors".to_owned(),
            Value::String(Value::Object(colors).to_string()),
        ),
    ]);
    Value::Object(document).to_string()
}

/// What a pushed `data` string asks for. Absent or `null` `tags`, `peers` and `tag_colors`
/// are empty (the Sciter UI starts from `{tags: [], peers: []}`); a wrongly typed one is
/// refused. A colour for a name not in `tags` is ignored, a tag without a colour has none
/// (the book then keeps or defaults it).
pub fn parse(data: &str) -> Result<BookReplacement, ClientApiError> {
    let Ok(Value::Object(mut document)) = serde_json::from_str::<Value>(data) else {
        return Err(invalid());
    };
    let names = names(document.remove("tags"))?;
    let colors = colors(document.remove("tag_colors"))?;
    let tags = names
        .into_iter()
        .map(|name| ReplacementTag {
            color: colors.get(&name).copied(),
            name,
        })
        .collect();
    let peers = match document.remove("peers") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| PeerPatch::from_json(item).map_err(ab_error))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(invalid()),
    };
    Ok(BookReplacement { peers, tags })
}

fn names(value: Option<Value>) -> Result<Vec<String>, ClientApiError> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(invalid()),
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::String(name) => Ok(name),
            _ => Err(invalid()),
        })
        .collect()
}

/// `tag_colors`: a JSON string holding `{name: int}` (what the client writes); a plain
/// object is accepted too. Values must be integers; their range is checked with the tag.
fn colors(value: Option<Value>) -> Result<std::collections::HashMap<String, i64>, ClientApiError> {
    let map = match value {
        None | Some(Value::Null) => return Ok(Default::default()),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ => return Err(invalid()),
        },
        Some(Value::Object(map)) => map,
        Some(_) => return Err(invalid()),
    };
    map.into_iter()
        .map(|(name, color)| Ok((name, color.as_i64().ok_or_else(invalid)?)))
        .collect()
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_DATA.into())
}

#[cfg(test)]
mod tests {
    use deskhaven_core::addressbooks::{Peer, PeerSecret, Tag};
    use deskhaven_db::time;
    use serde_json::json;

    use super::*;

    fn contents() -> BookContents {
        let peer = Peer {
            id: "1".to_owned(),
            alias: "a".to_owned(),
            note: String::new(),
            username: "u".to_owned(),
            hostname: String::new(),
            platform: String::new(),
            rdp_port: String::new(),
            rdp_username: String::new(),
            login_name: String::new(),
            tags: vec!["t".to_owned()],
            force_always_relay: false,
            secret: PeerSecret::Hash("h".to_owned()),
            extra: Map::new(),
        };
        BookContents {
            peers: vec![peer],
            tags: vec![Tag {
                name: "t".to_owned(),
                color: 0xFF00_00FF,
            }],
            updated_at: time::now(),
        }
    }

    #[test]
    fn render_writes_tag_colors_as_a_json_string_of_a_name_to_int_map() {
        let rendered: Value = serde_json::from_str(&render(&contents())).unwrap();

        assert_eq!(rendered["tags"], json!(["t"]));
        assert_eq!(rendered["peers"][0]["hash"], "h");
        assert_eq!(rendered["peers"][0]["tags"], json!(["t"]));
        let colors = rendered["tag_colors"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(colors).unwrap(),
            json!({ "t": 4_278_190_335_i64 })
        );
    }

    #[test]
    fn parse_pairs_colours_with_tags_and_ignores_colours_of_other_names() {
        let data = json!({
            "tags": ["a", "b"], "peers": [{ "id": "1" }],
            "tag_colors": json!({ "a": 7, "zzz": 9 }).to_string()
        })
        .to_string();

        let parsed = parse(&data).unwrap();

        let tags: Vec<_> = parsed
            .tags
            .iter()
            .map(|t| (t.name.as_str(), t.color))
            .collect();
        assert_eq!(tags, [("a", Some(7)), ("b", None)]);
        assert_eq!(parsed.peers[0].id, "1");
    }

    #[test]
    fn parse_takes_a_missing_or_null_section_as_empty_and_an_object_of_colours() {
        let sciter = parse(r#"{"tags":[],"peers":null}"#).unwrap();
        let object = parse(r#"{"tags":["a"],"tag_colors":{"a":3}}"#).unwrap();

        assert!(sciter.peers.is_empty() && sciter.tags.is_empty());
        assert_eq!(object.tags[0].color, Some(3));
    }

    #[test]
    fn parse_refuses_what_is_not_a_document() {
        for bad in [
            "",
            "null",
            "[]",
            "[[],[],\"\"]",
            "5",
            "{",
            r#"{"tags":"a"}"#,
            r#"{"tags":[1]}"#,
            r#"{"peers":{}}"#,
            r#"{"peers":[5]}"#,
            r#"{"tag_colors":5}"#,
            r#"{"tag_colors":"[]"}"#,
            r#"{"tag_colors":"{\"a\":\"x\"}"}"#,
            r#"{"tag_colors":"{\"a\":1.5}"}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }
}
