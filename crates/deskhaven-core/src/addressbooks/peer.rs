//! Wire rules: `forceAlwaysRelay` travels as the string `"true"` / `"false"`; unknown keys
//! are kept verbatim in `extra` and echoed back, so fields a newer client adds survive a round
//! trip; `null` for a known key means "not given".

use std::collections::HashSet;
use std::fmt;

use serde_json::{Map, Value};
use zeroize::Zeroizing;

use super::error::{AddressBookError, invalid};
use crate::text::{self, NOTE_MAX_LEN};

const ID_MAX_LEN: usize = 128;
const SHORT_TEXT_MAX_LEN: usize = 255;
const HASH_MAX_LEN: usize = 512;
const PASSWORD_MAX_LEN: usize = 256;
pub const TAG_NAME_MAX_LEN: usize = 64;
const MAX_PEER_TAGS: usize = 100;
pub const EXTRA_MAX_BYTES: usize = 8 * 1024;

const ID: &str = "id";
const HASH: &str = "hash";
const PASSWORD: &str = "password";
const USERNAME: &str = "username";
const HOSTNAME: &str = "hostname";
const PLATFORM: &str = "platform";
const ALIAS: &str = "alias";
const TAGS: &str = "tags";
const FORCE_ALWAYS_RELAY: &str = "forceAlwaysRelay";
const RDP_PORT: &str = "rdpPort";
const RDP_USERNAME: &str = "rdpUsername";
const LOGIN_NAME: &str = "loginName";
const NOTE: &str = "note";
/// Server-derived in RustDesk Pro: ignored on input, never stored.
const SERVER_DERIVED: [&str; 2] = ["device_group_name", "same_server"];
const TRUE: &str = "true";
const FALSE: &str = "false";

/// `None` leaves a field as it is (update) or empty (add).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PeerPatch {
    pub id: String,
    pub alias: Option<String>,
    pub note: Option<String>,
    pub username: Option<String>,
    pub hostname: Option<String>,
    pub platform: Option<String>,
    pub rdp_port: Option<String>,
    pub rdp_username: Option<String>,
    pub login_name: Option<String>,
    pub tags: Option<Vec<String>>,
    pub force_always_relay: Option<bool>,
    pub hash: Option<String>,
    pub password: Option<Zeroizing<String>>,
    pub extra: Map<String, Value>,
}

impl fmt::Debug for PeerPatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerPatch")
            .field("id", &self.id)
            .field("tags", &self.tags)
            .field("has_hash", &self.hash.is_some())
            .field("has_password", &self.password.is_some())
            .field("extra_keys", &self.extra.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PeerSecret {
    Hash(String),
    Password(Zeroizing<String>),
}

impl fmt::Debug for PeerSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(_) => f.write_str("Hash(..)"),
            Self::Password(_) => f.write_str("Password(..)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    pub alias: String,
    pub note: String,
    pub username: String,
    pub hostname: String,
    pub platform: String,
    pub rdp_port: String,
    pub rdp_username: String,
    pub login_name: String,
    pub tags: Vec<String>,
    pub force_always_relay: bool,
    pub secret: PeerSecret,
    pub extra: Map<String, Value>,
}

impl Peer {
    pub fn to_json(&self) -> Map<String, Value> {
        let mut out = self.extra.clone();
        let text = [
            (ID, &self.id),
            (ALIAS, &self.alias),
            (NOTE, &self.note),
            (USERNAME, &self.username),
            (HOSTNAME, &self.hostname),
            (PLATFORM, &self.platform),
            (RDP_PORT, &self.rdp_port),
            (RDP_USERNAME, &self.rdp_username),
            (LOGIN_NAME, &self.login_name),
        ];
        for (key, value) in text {
            out.insert(key.to_owned(), Value::String(value.clone()));
        }
        let tags = self.tags.iter().cloned().map(Value::String).collect();
        out.insert(TAGS.to_owned(), Value::Array(tags));
        let relay = if self.force_always_relay { TRUE } else { FALSE };
        out.insert(
            FORCE_ALWAYS_RELAY.to_owned(),
            Value::String(relay.to_owned()),
        );
        let (key, secret) = match &self.secret {
            PeerSecret::Hash(hash) => (HASH, hash.clone()),
            PeerSecret::Password(password) => (PASSWORD, password.as_str().to_owned()),
        };
        out.insert(key.to_owned(), Value::String(secret));
        out
    }
}

const CLIENT_TEXT_KEYS: [&str; 7] = [
    ALIAS,
    USERNAME,
    HOSTNAME,
    PLATFORM,
    RDP_PORT,
    RDP_USERNAME,
    LOGIN_NAME,
];

pub fn client_tag_name(raw: &str) -> String {
    text::strip_bidi_and_control(raw, usize::MAX, false)
}

impl PeerPatch {
    /// Control and bidi characters are stripped instead of refused, so one Hebrew or Arabic
    /// hostname (LRM / RLM) does not fail the whole sync. `id`, `hash` and `password` are still
    /// refused: stripping would silently turn them into other values.
    pub fn from_client_json(mut value: Value) -> Result<Self, AddressBookError> {
        if let Value::Object(map) = &mut value {
            for key in CLIENT_TEXT_KEYS {
                strip_string(map, key, false);
            }
            strip_string(map, NOTE, true);
            if let Some(Value::Array(tags)) = map.get_mut(TAGS) {
                for tag in tags {
                    strip_value(tag, false);
                }
            }
        }
        Self::from_json(value)
    }

    pub fn from_json(value: Value) -> Result<Self, AddressBookError> {
        let Value::Object(mut map) = value else {
            return Err(invalid("peer", "must be a JSON object"));
        };
        let id = take_string(&mut map, ID)?.ok_or_else(|| invalid(ID, "is required"))?;
        check_id(&id)?;
        let short =
            |map: &mut Map<String, Value>, key| take_text(map, key, SHORT_TEXT_MAX_LEN, false);
        let patch = Self {
            id,
            alias: short(&mut map, ALIAS)?,
            note: take_text(&mut map, NOTE, NOTE_MAX_LEN, true)?,
            username: short(&mut map, USERNAME)?,
            hostname: short(&mut map, HOSTNAME)?,
            platform: short(&mut map, PLATFORM)?,
            rdp_port: short(&mut map, RDP_PORT)?,
            rdp_username: short(&mut map, RDP_USERNAME)?,
            login_name: short(&mut map, LOGIN_NAME)?,
            tags: take_tags(&mut map)?,
            force_always_relay: take_relay(&mut map)?,
            hash: take_text(&mut map, HASH, HASH_MAX_LEN, false)?,
            password: take_password(&mut map)?,
            extra: Map::new(),
        };
        for key in SERVER_DERIVED {
            map.remove(key);
        }
        check_extra(&map)?;
        Ok(Self {
            extra: map,
            ..patch
        })
    }
}

fn strip_string(map: &mut Map<String, Value>, key: &str, multiline: bool) {
    if let Some(value) = map.get_mut(key) {
        strip_value(value, multiline);
    }
}

fn strip_value(value: &mut Value, multiline: bool) {
    if let Value::String(raw) = value {
        *raw = text::strip_bidi_and_control(raw, usize::MAX, multiline);
    }
}

/// URL clients normalise `.` and `..` away even percent-encoded, so such a name would address
/// another resource.
fn is_dot_segment(name: &str) -> bool {
    name == "." || name == ".."
}

pub fn check_id(id: &str) -> Result<(), AddressBookError> {
    let bad = |c: char| c.is_whitespace() || c.is_control();
    if id.is_empty() || id.chars().count() > ID_MAX_LEN || id.chars().any(bad) {
        return Err(invalid(
            ID,
            format!("must be 1-{ID_MAX_LEN} characters without spaces"),
        ));
    }
    if is_dot_segment(id) {
        return Err(invalid(ID, "must not be . or .."));
    }
    Ok(())
}

pub fn check_tag_name(field: &'static str, name: &str) -> Result<(), AddressBookError> {
    if name.is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    if is_dot_segment(name) {
        return Err(invalid(field, "must not be . or .."));
    }
    match text::problem(name, TAG_NAME_MAX_LEN, false) {
        Some(reason) => Err(invalid(field, reason)),
        None => Ok(()),
    }
}

fn take_string(
    map: &mut Map<String, Value>,
    key: &'static str,
) -> Result<Option<String>, AddressBookError> {
    match map.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(invalid(key, "must be a string")),
    }
}

fn take_text(
    map: &mut Map<String, Value>,
    key: &'static str,
    max: usize,
    multiline: bool,
) -> Result<Option<String>, AddressBookError> {
    let value = take_string(map, key)?;
    if let Some(reason) = value
        .as_deref()
        .and_then(|v| text::problem(v, max, multiline))
    {
        return Err(invalid(key, reason));
    }
    Ok(value)
}

fn take_password(
    map: &mut Map<String, Value>,
) -> Result<Option<Zeroizing<String>>, AddressBookError> {
    let value = take_string(map, PASSWORD)?.map(Zeroizing::new);
    if value
        .as_ref()
        .is_some_and(|p| p.chars().count() > PASSWORD_MAX_LEN)
    {
        return Err(invalid(
            PASSWORD,
            format!("must be at most {PASSWORD_MAX_LEN} characters"),
        ));
    }
    Ok(value)
}

fn take_tags(map: &mut Map<String, Value>) -> Result<Option<Vec<String>>, AddressBookError> {
    let items = match map.remove(TAGS) {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(invalid(TAGS, "must be a list of strings")),
    };
    if items.len() > MAX_PEER_TAGS {
        return Err(invalid(TAGS, format!("at most {MAX_PEER_TAGS} tags")));
    }
    let mut tags: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        let Value::String(tag) = item else {
            return Err(invalid(TAGS, "must be a list of strings"));
        };
        check_tag_name(TAGS, &tag)?;
        tags.push(tag);
    }
    Ok(Some(unique(tags)))
}

pub(crate) fn unique(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

/// Another string reads as `false`, as the client reads it.
fn take_relay(map: &mut Map<String, Value>) -> Result<Option<bool>, AddressBookError> {
    match map.remove(FORCE_ALWAYS_RELAY) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s == TRUE)),
        Some(Value::Bool(b)) => Ok(Some(b)),
        Some(_) => Err(invalid(FORCE_ALWAYS_RELAY, "must be \"true\" or \"false\"")),
    }
}

pub(crate) fn check_extra(extra: &Map<String, Value>) -> Result<(), AddressBookError> {
    let size = serde_json::to_vec(extra).map_or(usize::MAX, |bytes| bytes.len());
    if size > EXTRA_MAX_BYTES {
        return Err(invalid(
            "peer",
            format!("unknown fields must total at most {EXTRA_MAX_BYTES} bytes"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> Result<PeerPatch, AddressBookError> {
        PeerPatch::from_json(value)
    }

    #[test]
    fn contract_example_parses_into_known_fields() {
        let patch = parse(json!({
            "id": "123", "hash": "h", "password": "p", "username": "u", "hostname": "host",
            "platform": "Linux", "alias": "a", "tags": ["x"], "forceAlwaysRelay": "true",
            "rdpPort": "3389", "rdpUsername": "r", "loginName": "l",
            "device_group_name": "g", "note": "n", "same_server": true
        }))
        .unwrap();

        assert_eq!(patch.id, "123");
        assert_eq!(patch.alias.as_deref(), Some("a"));
        assert_eq!(patch.rdp_port.as_deref(), Some("3389"));
        assert_eq!(patch.login_name.as_deref(), Some("l"));
        assert_eq!(patch.force_always_relay, Some(true));
        assert_eq!(patch.tags, Some(vec!["x".to_owned()]));
        assert_eq!(patch.hash.as_deref(), Some("h"));
        assert_eq!(patch.password.as_deref().map(String::as_str), Some("p"));
        assert!(patch.extra.is_empty(), "{:?}", patch.extra);
    }

    #[test]
    fn unknown_keys_go_to_extra_verbatim() {
        let patch = parse(json!({ "id": "1", "futureField": {"a": [1, null]}, "x": 2 })).unwrap();

        assert_eq!(patch.extra["futureField"], json!({"a": [1, null]}));
        assert_eq!(patch.extra["x"], json!(2));
    }

    #[test]
    fn force_always_relay_accepts_the_client_strings_and_booleans() {
        let relay = |v: Value| parse(json!({ "id": "1", "forceAlwaysRelay": v })).unwrap();

        assert_eq!(relay(json!("true")).force_always_relay, Some(true));
        assert_eq!(relay(json!("false")).force_always_relay, Some(false));
        assert_eq!(relay(json!("yes")).force_always_relay, Some(false));
        assert_eq!(relay(json!(true)).force_always_relay, Some(true));
        assert_eq!(relay(Value::Null).force_always_relay, None);
        assert!(parse(json!({ "id": "1", "forceAlwaysRelay": 1 })).is_err());
    }

    #[test]
    fn rejects_bad_ids_types_and_sizes() {
        for bad in [
            json!([]),
            json!({}),
            json!({ "id": "" }),
            json!({ "id": "a b" }),
            json!({ "id": 5 }),
            json!({ "id": "x".repeat(129) }),
            json!({ "id": "1", "alias": 3 }),
            json!({ "id": "1", "alias": "a\u{0}" }),
            json!({ "id": "1", "alias": "\u{202E}nimda" }),
            json!({ "id": "1", "note": "x\u{2066}y" }),
            json!({ "id": "1", "tags": ["\u{200F}t"] }),
            json!({ "id": "1", "tags": "a" }),
            json!({ "id": "1", "tags": [1] }),
            json!({ "id": "1", "tags": [""] }),
            json!({ "id": "1", "password": "p".repeat(257) }),
            json!({ "id": "1", "big": "x".repeat(EXTRA_MAX_BYTES) }),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn client_peers_are_stripped_of_control_and_bidi_characters_not_refused() {
        let patch = PeerPatch::from_client_json(json!({
            "id": "1",
            "hostname": "\u{05E9}\u{200F}\u{05DC}\u{05D5}\u{05DD}",
            "alias": "\u{202E}nimda\u{0}",
            "username": "u\u{200E}", "platform": "Li\u{2066}nux", "rdpPort": "33\u{7}89",
            "rdpUsername": "r\u{061C}", "loginName": "l\u{202A}",
            "note": "l1\n\u{202E}l2\t\u{0}",
            "tags": ["\u{200F}t", "a\u{200D}b"],
        }))
        .unwrap();

        assert_eq!(
            patch.hostname.as_deref(),
            Some("\u{05E9}\u{05DC}\u{05D5}\u{05DD}")
        );
        assert_eq!(patch.alias.as_deref(), Some("nimda"));
        assert_eq!(patch.username.as_deref(), Some("u"));
        assert_eq!(patch.platform.as_deref(), Some("Linux"));
        assert_eq!(patch.rdp_port.as_deref(), Some("3389"));
        assert_eq!(patch.rdp_username.as_deref(), Some("r"));
        assert_eq!(patch.login_name.as_deref(), Some("l"));
        assert_eq!(patch.note.as_deref(), Some("l1\nl2\t"));
        assert_eq!(
            patch.tags,
            Some(vec!["t".to_owned(), "a\u{200D}b".to_owned()])
        );
    }

    #[test]
    fn client_peers_still_refuse_what_stripping_would_change_in_meaning() {
        for bad in [
            json!({ "id": "a\u{0}b" }),
            json!({ "id": "1", "hash": "h\u{202E}" }),
            json!({ "id": "1", "alias": 3 }),
            json!({ "id": "1", "alias": "x".repeat(256) }),
            json!({ "id": "1", "tags": ["."] }),
            json!({ "id": "1", "tags": ["\u{200F}"] }),
        ] {
            assert!(PeerPatch::from_client_json(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn client_tag_names_lose_control_and_bidi_characters() {
        assert_eq!(client_tag_name("\u{202E}la\u{0}b\u{200F}el"), "label");
    }

    #[test]
    fn duplicate_tags_collapse_and_note_may_span_lines() {
        let patch = parse(json!({ "id": "1", "tags": ["a", "b", "a"], "note": "l1\nl2" })).unwrap();

        assert_eq!(patch.tags, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(patch.note.as_deref(), Some("l1\nl2"));
    }

    #[test]
    fn to_json_emits_the_relay_string_and_one_secret_and_extra() {
        let peer = Peer {
            id: "1".to_owned(),
            alias: String::new(),
            note: String::new(),
            username: String::new(),
            hostname: String::new(),
            platform: String::new(),
            rdp_port: String::new(),
            rdp_username: String::new(),
            login_name: String::new(),
            tags: vec!["t".to_owned()],
            force_always_relay: true,
            secret: PeerSecret::Password(Zeroizing::new("pw".to_owned())),
            extra: Map::from_iter([("future".to_owned(), json!(1))]),
        };

        let out = Value::Object(peer.to_json());

        assert_eq!(out["forceAlwaysRelay"], json!("true"));
        assert_eq!(out["password"], json!("pw"));
        assert!(out.get("hash").is_none());
        assert_eq!(out["future"], json!(1));
        assert_eq!(out["tags"], json!(["t"]));
    }

    #[test]
    fn debug_never_shows_secrets() {
        let patch = parse(json!({ "id": "1", "password": "hunter2", "hash": "h4sh" })).unwrap();

        let shown = format!("{patch:?}");

        assert!(
            !shown.contains("hunter2") && !shown.contains("h4sh"),
            "{shown}"
        );
    }
}
