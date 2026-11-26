//! `tag_colors` is a JSON string again inside the document string: `{name: ARGB int}`.
//! Peers use the per-item API's codec, so every field both APIs share has one definition;
//! a legacy client ignores fields it does not know, and a push without them leaves the stored
//! values alone.

use deskhaven_core::addressbooks::BookContents;
use serde_json::{Map, Value};

/// Peers by id and tags by name: a legacy client's own tag order is not kept.
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
}
