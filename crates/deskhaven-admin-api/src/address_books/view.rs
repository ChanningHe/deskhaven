//! Wire types of the address book endpoints. Timestamps are unix milliseconds; colours are
//! the client's 32-bit ARGB integers.

use std::fmt;

use deskhaven_core::addressbooks::{
    BookSummary, ImportReport, MAX_IMPORT_DEVICES, NamedShare, Peer, PeerSecret, Rule, Subject, Tag,
};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use utoipa::ToSchema;

/// Highest ARGB value (32 bits).
const MAX_COLOR: i64 = 0xFFFF_FFFF;
/// Most ids or names one batch request carries.
const MAX_BATCH: usize = 1000;

/// A shared book with the sizes of its parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BookView {
    /// The identifier clients use.
    pub guid: String,
    pub name: String,
    pub note: String,
    /// `null`: managed by the administrators only.
    pub owner_user_id: Option<i64>,
    pub owner: Option<String>,
    pub peer_count: u64,
    pub tag_count: u64,
    pub share_count: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<BookSummary> for BookView {
    fn from(s: BookSummary) -> Self {
        Self {
            guid: s.book.guid,
            name: s.book.name,
            note: s.book.note,
            owner_user_id: s.book.owner_user_id,
            owner: s.owner,
            peer_count: s.peer_count,
            tag_count: s.tag_count,
            share_count: s.share_count,
            created_at: s.book.created_at.timestamp_millis(),
            updated_at: s.book.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewBookBody {
    /// 1-64 characters; unique among shared books (case-insensitive) and not a name the
    /// client reserves for its own entries.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub note: String,
    /// The user who owns the book (full control); absent: managed by the administrators.
    #[garde(skip)]
    pub owner_user_id: Option<i64>,
}

/// Changes to a book; absent fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BookPatch {
    #[garde(length(chars, min = 1, max = 64))]
    pub name: Option<String>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
}

/// What a share is granted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubjectKind {
    User,
    Group,
}

impl SubjectKind {
    pub fn subject(self, id: i64) -> Subject {
        match self {
            Self::User => Subject::User(id),
            Self::Group => Subject::Group(id),
        }
    }
}

/// One grant of a book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ShareView {
    pub subject_type: SubjectKind,
    pub subject_id: i64,
    /// Username or group name; `null` when the subject is gone.
    pub subject_name: Option<String>,
    /// 1 read-only, 2 read-write, 3 full control (manage shares up to 2).
    pub rule: u8,
}

impl From<NamedShare> for ShareView {
    fn from(n: NamedShare) -> Self {
        let (subject_type, subject_id) = match n.share.subject {
            Subject::User(id) => (SubjectKind::User, id),
            Subject::Group(id) => (SubjectKind::Group, id),
        };
        Self {
            subject_type,
            subject_id,
            subject_name: n.subject_name,
            rule: rule_number(n.share.rule),
        }
    }
}

/// The number clients and the console use for a rule.
pub fn rule_number(rule: Rule) -> u8 {
    match rule {
        Rule::ReadOnly => 1,
        Rule::ReadWrite => 2,
        Rule::FullControl => 3,
    }
}

/// The rule behind `number` (1-3).
pub fn rule_from_number(number: u8) -> Option<Rule> {
    match number {
        1 => Some(Rule::ReadOnly),
        2 => Some(Rule::ReadWrite),
        3 => Some(Rule::FullControl),
        _ => None,
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareBody {
    #[garde(skip)]
    pub subject_type: SubjectKind,
    #[garde(skip)]
    pub subject_id: i64,
    /// 1 read-only, 2 read-write, 3 full control. A read-only grant hands the sharee's client
    /// every entry's password.
    #[garde(range(min = 1, max = 3))]
    pub rule: u8,
}

/// A list that is not paged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ItemList<T> {
    pub items: Vec<T>,
}

/// An entry. The password is write-only: only whether there is one is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct PeerView {
    /// The RustDesk id.
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
    pub has_password: bool,
}

impl From<Peer> for PeerView {
    fn from(p: Peer) -> Self {
        let has_password = matches!(&p.secret, PeerSecret::Password(pw) if !pw.is_empty());
        Self {
            id: p.id,
            alias: p.alias,
            note: p.note,
            username: p.username,
            hostname: p.hostname,
            platform: p.platform,
            rdp_port: p.rdp_port,
            rdp_username: p.rdp_username,
            login_name: p.login_name,
            tags: p.tags,
            force_always_relay: p.force_always_relay,
            has_password,
        }
    }
}

/// Entry fields, to create (`id` required) or to change (absent fields stay as they are;
/// `id`, when given, must be the entry's). `password` is write-only; an empty one removes
/// the stored password. Limits are the core service's (reported as `validation_failed`).
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PeerBody {
    #[garde(length(chars, min = 1, max = 128))]
    pub id: Option<String>,
    #[garde(length(chars, max = 255))]
    pub alias: Option<String>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
    #[garde(length(chars, max = 255))]
    pub username: Option<String>,
    #[garde(length(chars, max = 255))]
    pub hostname: Option<String>,
    #[garde(length(chars, max = 255))]
    pub platform: Option<String>,
    #[garde(length(chars, max = 255))]
    pub rdp_port: Option<String>,
    #[garde(length(chars, max = 255))]
    pub rdp_username: Option<String>,
    #[garde(length(chars, max = 255))]
    pub login_name: Option<String>,
    #[garde(skip)]
    pub tags: Option<Vec<String>>,
    #[garde(skip)]
    pub force_always_relay: Option<bool>,
    #[garde(length(chars, max = 256))]
    pub password: Option<String>,
}

impl fmt::Debug for PeerBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerBody")
            .field("id", &self.id)
            .field("has_password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

impl PeerBody {
    /// The client-format peer object for entry `id` (what `PeerPatch::from_json` parses).
    pub fn to_json(&self, id: &str) -> Value {
        let text = [
            ("alias", &self.alias),
            ("note", &self.note),
            ("username", &self.username),
            ("hostname", &self.hostname),
            ("platform", &self.platform),
            ("rdpPort", &self.rdp_port),
            ("rdpUsername", &self.rdp_username),
            ("loginName", &self.login_name),
            ("password", &self.password),
        ];
        let mut out = Map::new();
        out.insert("id".to_owned(), id.into());
        for (key, value) in text {
            if let Some(value) = value {
                out.insert(key.to_owned(), value.as_str().into());
            }
        }
        if let Some(tags) = &self.tags {
            out.insert("tags".to_owned(), tags.clone().into());
        }
        if let Some(relay) = self.force_always_relay {
            out.insert("forceAlwaysRelay".to_owned(), relay.into());
        }
        Value::Object(out)
    }
}

/// Entries to remove; ids not in the book are ignored.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IdsBody {
    #[garde(length(max = MAX_BATCH))]
    pub ids: Vec<String>,
}

/// Devices to import (`devices.id`).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportDevicesBody {
    #[garde(length(min = 1, max = MAX_IMPORT_DEVICES))]
    pub device_ids: Vec<i64>,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ImportDevicesView {
    /// RustDesk ids of the entries added.
    pub added: Vec<String>,
    /// RustDesk ids the book already listed (left as they are).
    pub existing: Vec<String>,
    /// `devices.id` values that name no device.
    pub missing: Vec<i64>,
}

impl From<ImportReport> for ImportDevicesView {
    fn from(r: ImportReport) -> Self {
        Self {
            added: r.added,
            existing: r.existing,
            missing: r.missing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TagView {
    pub name: String,
    /// ARGB, 32 bits.
    pub color: i64,
}

impl From<Tag> for TagView {
    fn from(t: Tag) -> Self {
        Self {
            name: t.name,
            color: t.color,
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewTagBody {
    /// 1-64 characters, unique in the book.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    /// ARGB; default the client's grey.
    #[garde(inner(range(min = 0, max = MAX_COLOR)))]
    pub color: Option<i64>,
}

/// Rename and / or recolour a tag; at least one. Renaming also renames it on the entries.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TagPatch {
    #[garde(length(chars, min = 1, max = 64))]
    pub name: Option<String>,
    #[garde(inner(range(min = 0, max = MAX_COLOR)))]
    pub color: Option<i64>,
}

/// Tags to delete (also from the entries); unknown names are ignored.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NamesBody {
    #[garde(length(max = MAX_BATCH))]
    pub names: Vec<String>,
}
