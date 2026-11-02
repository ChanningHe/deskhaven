//! Reading a pushed document (see [`super::document`]) as a stream.
//!
//! A push is authenticated but otherwise untrusted, and the document is up to
//! [`MAX_LEGACY_AB_BODY`](super::MAX_LEGACY_AB_BODY) bytes of small elements: building a
//! `serde_json::Value` tree of it and only then counting costs tens of times the body in
//! memory (64k `{"id":"1"}` peers are 1 MB of text and about 80 MB of tree). So the caps
//! are enforced while reading:
//!
//! - the element after the `max_peers`-th peer is not read, the push is refused (the count
//!   is of raw elements: a client that sends more than the book holds is wrong);
//! - the same for more than [`MAX_BOOK_TAGS`] tags and for more than that many colours;
//! - one peer is read as raw text first and refused above [`MAX_PEER_BYTES`], so the tree
//!   built for it (then dropped) is small, and only the checked [`PeerPatch`] is kept;
//! - unknown top-level keys are skipped without building anything.
//!
//! Why a push stopped is passed out through a side channel (serde errors are text): see
//! [`Stash`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use deskhaven_core::addressbooks::{
    AddressBookError, BookReplacement, MAX_BOOK_TAGS, PeerPatch, ReplacementTag,
};
use serde::Deserializer as _;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use crate::ClientApiError;
use crate::ab::ab_error;

const INVALID_DATA: &str = "Invalid data";
/// Largest raw JSON of one peer: id 128 + four 255-character texts + hash 512 + note 2000
/// + 100 tags of 64 characters + 8 KiB of unknown fields stay below this with their
///   escapes and punctuation.
const MAX_PEER_BYTES: usize = 32 * 1024;
const MAX_TAGS: usize = MAX_BOOK_TAGS as usize;

/// The reason a read stopped, set right before the visitor returns its (textual) serde
/// error. `None` after a failed read means malformed JSON or a wrongly typed value.
type Stash<'a> = &'a RefCell<Option<ClientApiError>>;

/// What a pushed `data` string asks for. Absent or `null` `tags`, `peers` and `tag_colors`
/// are empty (the Sciter UI starts from `{tags: [], peers: []}`); a wrongly typed one is
/// refused. A colour for a name not in `tags` is ignored, a tag without a colour has none
/// (the book then keeps or defaults it). More than `max_peers` peers (counting repeated
/// ids) is [`AddressBookError::Full`], as for the per-item API.
pub fn parse(data: &str, max_peers: u32) -> Result<BookReplacement, ClientApiError> {
    let stash = RefCell::new(None);
    let parts = read(data, max_peers, &stash).map_err(|_| {
        stash
            .take()
            .unwrap_or_else(|| ClientApiError::BadRequest(INVALID_DATA.into()))
    })?;
    let tags = parts
        .tags
        .into_iter()
        .map(|name| ReplacementTag {
            color: parts.colors.get(&name).copied(),
            name,
        })
        .collect();
    Ok(BookReplacement {
        peers: parts.peers,
        tags,
    })
}

#[derive(Default)]
struct Parts {
    tags: Vec<String>,
    peers: Vec<PeerPatch>,
    colors: HashMap<String, i64>,
}

fn read(data: &str, max_peers: u32, stash: Stash) -> Result<Parts, serde_json::Error> {
    let mut json = serde_json::Deserializer::from_str(data);
    let visitor = Document { max_peers, stash };
    let parts = json.deserialize_map(visitor)?;
    json.end()?;
    Ok(parts)
}

/// Stops the read for `reason`.
fn abort<E: de::Error>(stash: Stash, reason: ClientApiError) -> E {
    *stash.borrow_mut() = Some(reason);
    E::custom("push refused")
}

fn too_many_tags() -> ClientApiError {
    let reason = format!("an address book holds at most {MAX_BOOK_TAGS} tags");
    ab_error(AddressBookError::Invalid {
        field: "tags",
        reason,
    })
}

#[derive(Clone, Copy)]
struct Document<'a> {
    max_peers: u32,
    stash: Stash<'a>,
}

impl<'de> Visitor<'de> for Document<'_> {
    type Value = Parts;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Parts, A::Error> {
        let mut parts = Parts::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "tags" => parts.tags = map.next_value_seed(Nullable(Names(self.stash)))?,
                "peers" => parts.peers = map.next_value_seed(Nullable(Peers(self)))?,
                "tag_colors" => parts.colors = map.next_value_seed(Nullable(Colors(self.stash)))?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(parts)
    }
}

/// `null` reads as the default; anything else goes to the inner seed.
struct Nullable<S>(S);

impl<'de, S> DeserializeSeed<'de> for Nullable<S>
where
    S: DeserializeSeed<'de>,
    S::Value: Default,
{
    type Value = S::Value;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<S::Value, D::Error> {
        struct V<S>(S);
        impl<'de, S> Visitor<'de> for V<S>
        where
            S: DeserializeSeed<'de>,
            S::Value: Default,
        {
            type Value = S::Value;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a value or null")
            }

            fn visit_none<E: de::Error>(self) -> Result<S::Value, E> {
                Ok(S::Value::default())
            }

            fn visit_unit<E: de::Error>(self) -> Result<S::Value, E> {
                Ok(S::Value::default())
            }

            fn visit_some<D: de::Deserializer<'de>>(self, de: D) -> Result<S::Value, D::Error> {
                self.0.deserialize(de)
            }
        }
        de.deserialize_option(V(self.0))
    }
}

/// `tags`: a list of at most [`MAX_BOOK_TAGS`] strings.
struct Names<'a>(Stash<'a>);

impl<'de> DeserializeSeed<'de> for Names<'_> {
    type Value = Vec<String>;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<Vec<String>, D::Error> {
        de.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Names<'_> {
    type Value = Vec<String>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of tag names")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<String>, A::Error> {
        let mut names = Vec::new();
        while names.len() < MAX_TAGS {
            match seq.next_element::<String>()? {
                Some(name) => names.push(name),
                None => return Ok(names),
            }
        }
        match seq.next_element::<IgnoredAny>()? {
            None => Ok(names),
            Some(_) => Err(abort(self.0, too_many_tags())),
        }
    }
}

/// `peers`: a list of at most `max_peers` peer objects.
struct Peers<'a>(Document<'a>);

impl<'de> DeserializeSeed<'de> for Peers<'_> {
    type Value = Vec<PeerPatch>;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<Vec<PeerPatch>, D::Error> {
        de.deserialize_seq(self)
    }
}

impl<'de> Visitor<'de> for Peers<'_> {
    type Value = Vec<PeerPatch>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a list of peers")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<PeerPatch>, A::Error> {
        let Document { max_peers, stash } = self.0;
        let mut peers = Vec::new();
        while peers.len() < max_peers as usize {
            match seq.next_element::<Box<RawValue>>()? {
                Some(raw) => peers.push(peer(&raw, stash)?),
                None => return Ok(peers),
            }
        }
        // The element after the last one the book can hold is not even read.
        match seq.next_element::<IgnoredAny>()? {
            None => Ok(peers),
            Some(_) => Err(abort(
                stash,
                ab_error(AddressBookError::Full { max: max_peers }),
            )),
        }
    }
}

/// One peer from its raw JSON: size-checked before the tree is built, validated by the
/// per-item API's own codec.
fn peer<E: de::Error>(raw: &RawValue, stash: Stash) -> Result<PeerPatch, E> {
    let invalid = |reason: String| {
        let error = AddressBookError::Invalid {
            field: "peer",
            reason,
        };
        abort(stash, ab_error(error))
    };
    if raw.get().len() > MAX_PEER_BYTES {
        return Err(invalid(format!("must be at most {MAX_PEER_BYTES} bytes")));
    }
    let value = serde_json::from_str(raw.get()).map_err(E::custom)?;
    PeerPatch::from_json(value).map_err(|err| abort(stash, ab_error(err)))
}

/// `tag_colors`: a JSON string holding `{name: int}` (what the client writes) or the object
/// itself, of at most [`MAX_BOOK_TAGS`] entries. Values must be integers; their range is
/// checked with the tag.
struct Colors<'a>(Stash<'a>);

impl<'de> DeserializeSeed<'de> for Colors<'_> {
    type Value = HashMap<String, i64>;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        de: D,
    ) -> Result<HashMap<String, i64>, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Colors<'_> {
    type Value = HashMap<String, i64>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON string or object of tag colours")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<HashMap<String, i64>, E> {
        let mut json = serde_json::Deserializer::from_str(text);
        let colors = json.deserialize_map(self).map_err(E::custom)?;
        json.end().map_err(E::custom)?;
        Ok(colors)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<HashMap<String, i64>, A::Error> {
        let mut colors = HashMap::new();
        while let Some(name) = map.next_key::<String>()? {
            if colors.len() >= MAX_TAGS {
                let reason = format!("at most {MAX_BOOK_TAGS} colours");
                let error = AddressBookError::Invalid {
                    field: "tag_colors",
                    reason,
                };
                return Err(abort(self.0, ab_error(error)));
            }
            colors.insert(name, map.next_value::<i64>()?);
        }
        Ok(colors)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const FULL: &str = "You have reached the maximum number of managed devices.";

    fn refusal(result: Result<BookReplacement, ClientApiError>) -> String {
        result.map(|_| ()).unwrap_err().to_string()
    }

    /// `n` minimal peers as JSON list items (no brackets).
    fn peers(n: usize) -> String {
        (0..n)
            .map(|i| format!(r#"{{"id":"{i}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
    }

    #[test]
    fn parse_pairs_colours_with_tags_and_ignores_colours_of_other_names() {
        let data = json!({
            "tags": ["a", "b"], "peers": [{ "id": "1" }],
            "tag_colors": json!({ "a": 7, "zzz": 9 }).to_string()
        })
        .to_string();

        let parsed = parse(&data, 10).unwrap();

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
        let sciter = parse(r#"{"tags":[],"peers":null,"tag_colors":null}"#, 10).unwrap();
        let object = parse(r#"{"tags":["a"],"tag_colors":{"a":3}}"#, 10).unwrap();

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
            r#"{"tag_colors":5}"#,
            r#"{"tag_colors":"[]"}"#,
            r#"{"tag_colors":"{\"a\":\"x\"}"}"#,
            r#"{"tag_colors":"{\"a\":1.5}"}"#,
            r#"{"tags":[]} trailing"#,
        ] {
            assert_eq!(refusal(parse(bad, 10)), "Invalid data", "{bad:?}");
        }
    }

    #[test]
    fn unknown_top_level_keys_are_skipped() {
        let data = format!(
            r#"{{"pad":"{}","peers":[{{"id":"1"}}]}}"#,
            "x".repeat(100_000)
        );

        assert_eq!(parse(&data, 10).unwrap().peers.len(), 1);
    }

    #[test]
    fn the_peer_after_the_cap_is_never_read() {
        // A fourth element is broken JSON and the third has a bad id; either would be a
        // different refusal if the element were built. The cap's refusal means the read
        // stopped right after skipping the third.
        let data = format!(r#"{{"peers":[{},{{"id":"bad id"}} , {{"#, peers(2));

        let result = parse(&data, 2);

        assert_eq!(refusal(result), FULL);
    }

    #[test]
    fn a_huge_peer_list_is_refused_at_the_cap_without_reading_the_rest() {
        let data = format!(r#"{{"peers":[{}]}}"#, peers(64_000));

        assert_eq!(refusal(parse(&data, 1000)), FULL);
        assert_eq!(parse(&data, 64_000).unwrap().peers.len(), 64_000);
    }

    #[test]
    fn repeated_ids_count_toward_the_cap_as_sent() {
        let data = r#"{"peers":[{"id":"1"},{"id":"1"},{"id":"1"}]}"#;

        assert_eq!(refusal(parse(data, 2)), FULL);
        assert_eq!(parse(data, 3).unwrap().peers.len(), 3);
    }

    #[test]
    fn tags_and_colours_stop_at_their_cap() {
        let names: Vec<_> = (0..=MAX_TAGS).map(|i| format!("\"t{i}\"")).collect();
        let tags = format!(r#"{{"tags":[{}, {{"#, names.join(","));
        let entries: Vec<_> = (0..=MAX_TAGS).map(|i| format!("\"t{i}\":1")).collect();
        let colors = json!({ "tag_colors": format!("{{{}", entries.join(",")) }).to_string();
        let ok_colors =
            json!({ "tag_colors": format!("{{{}}}", entries[..MAX_TAGS].join(",")) }).to_string();

        assert_eq!(
            refusal(parse(&tags, 10)),
            "Invalid tags: an address book holds at most 1000 tags"
        );
        assert_eq!(
            refusal(parse(&colors, 10)),
            "Invalid tag_colors: at most 1000 colours"
        );
        assert_eq!(parse(&ok_colors, 10).unwrap().tags.len(), 0);
    }

    #[test]
    fn a_peer_over_the_size_cap_is_refused_before_its_tree_is_built() {
        let big = format!(
            r#"{{"peers":[{{"id":"1","x":"{}"}}]}}"#,
            "y".repeat(MAX_PEER_BYTES)
        );

        assert_eq!(
            refusal(parse(&big, 10)),
            format!("Invalid peer: must be at most {MAX_PEER_BYTES} bytes")
        );
    }

    #[test]
    fn a_bad_peer_reports_the_per_item_api_wording() {
        let data = r#"{"peers":[{"id":"1"},{"alias":"no id"}]}"#;

        assert_eq!(refusal(parse(data, 10)), "Invalid id: is required");
        assert_eq!(
            refusal(parse(r#"{"peers":[5]}"#, 10)),
            "Invalid peer: must be a JSON object"
        );
    }
}
