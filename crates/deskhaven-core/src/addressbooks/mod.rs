//! Every operation checks the actor's rule here, whatever the caller already checked.
//!
//! Peer secrets are sealed with the master key: the personal-book `hash` is the salted
//! password the client connects with, a credential in its own right.
//!
//! Shared book names are kept unique (case-insensitively) and off [`RESERVED_NAMES`]: the
//! client keys books by name and would merge them.

mod access;
mod admin_books;
mod books;
mod contents;
mod error;
mod import;
mod peer;
mod peer_writes;
mod peers;
mod placement;
mod shared_books;
mod shares;
mod snapshot;
mod tags;

use std::sync::Arc;

use deskhaven_config::settings::ADDRESSBOOK_MAX_PEERS;
use deskhaven_db::entity::address_book;
use deskhaven_db::time::TimestampMs;
use sea_orm::DatabaseConnection;

pub use access::{Actor, covers};
pub use admin_books::{BookQuery, BookSort, BookSortField, BookSummary, NamedShare};
pub use contents::{BookContents, BookReplacement, DEFAULT_TAG_COLOR, ReplacementTag};
pub use deskhaven_db::entity::address_book::Kind;
pub use deskhaven_db::entity::address_book_share::Rule;
pub use error::AddressBookError;
pub use import::{ImportReport, MAX_IMPORT_DEVICES};
pub use peer::{EXTRA_MAX_BYTES, Peer, PeerPatch, PeerSecret, TAG_NAME_MAX_LEN, client_tag_name};
pub use shares::{Share, Subject};
pub use tags::MAX_BOOK_TAGS;

pub(crate) use shares::{forget_group, forget_user};

use crate::crypto::SecretBox;
use crate::settings::SettingsService;

/// Names the client gives its own entries; a shared book with one would replace that entry.
pub const RESERVED_NAMES: [&str; 2] = ["My address book", "Legacy address book"];
pub const PERSONAL_NAME: &str = RESERVED_NAMES[0];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Book {
    pub id: i64,
    pub guid: String,
    pub kind: Kind,
    /// `None` for an admin-managed shared book.
    pub owner_user_id: Option<i64>,
    pub name: String,
    pub note: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<address_book::Model> for Book {
    fn from(m: address_book::Model) -> Self {
        Self {
            id: m.id,
            guid: m.guid,
            kind: m.kind,
            owner_user_id: m.owner_user_id,
            name: m.name,
            note: m.note,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub book: Book,
    pub owner: Option<String>,
    pub rule: Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub color: i64,
}

#[derive(Clone, Debug)]
pub struct NewSharedBook {
    pub name: String,
    pub note: String,
    pub owner_user_id: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct SharedBookUpdate {
    pub name: Option<String>,
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct AddressBookService {
    db: DatabaseConnection,
    settings: SettingsService,
    crypto: Arc<SecretBox>,
}

impl AddressBookService {
    pub fn new(db: DatabaseConnection, settings: SettingsService, crypto: Arc<SecretBox>) -> Self {
        Self {
            db,
            settings,
            crypto,
        }
    }

    pub fn max_peers(&self) -> u32 {
        self.settings.snapshot().get(&ADDRESSBOOK_MAX_PEERS)
    }
}
