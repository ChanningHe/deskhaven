//! Address books: personal and shared books, shares, peers and tags (04 §5, 06 §4.2).
//!
//! - Every user has one personal book, created on first access ([`AddressBookService::personal`];
//!   concurrent first accesses converge on one row through the partial unique index).
//! - Shared books are created by an administrator ([`AddressBookService::create_shared`]),
//!   optionally owned by a user, and granted to users and groups with a [`Rule`]
//!   (1 read-only, 2 read-write, 3 full control). See [`Actor`] for who holds which rule.
//!   Reads need rule 1, peer and tag writes rule 2, share and book management rule 3; every
//!   operation checks it here, whatever the caller already checked.
//! - Peers: personal books keep the client's password `hash` and never a `password`; shared
//!   books keep the `password` and never a `hash`. Both are sealed with the master key (AAD
//!   context `address_book_peers.<hash_enc|password_enc>:<book id>:<rustdesk id>`): the
//!   hash is the salted password the client connects with (`src/client.rs:4777-4799`), a
//!   credential in its own right. Unknown peer fields are kept in `extra` and echoed back
//!   (see [`PeerPatch`]).
//! - Capacity: a book holds at most `addressbook.max_peers` peers (read at each add).
//! - Whole-book operations ([`AddressBookService::contents`] and
//!   [`AddressBookService::replace_contents`]) serve legacy clients that read and push the
//!   book as one document; the merge rules are in the `contents` module.
//! - Every write locks the book row, so writes to one book are serialized on both backends.
//!
//! Names: shared book names are not unique in the schema. This service refuses to create
//! or rename a shared book to a name another shared book has (compared case-insensitively)
//! or to a name the client reserves for its own entries ([`RESERVED_NAMES`]): the client
//! keys books by name and would merge them. Resolving by name
//! ([`AddressBookService::find_shared_by_name`]) never guesses: several matches (possible
//! after a PostgreSQL race between two creations) are [`AddressBookError::AmbiguousName`].

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
pub use peer::{EXTRA_MAX_BYTES, Peer, PeerPatch, PeerSecret, TAG_NAME_MAX_LEN};
pub use shares::{Share, Subject};
pub use tags::MAX_BOOK_TAGS;

pub(crate) use shares::{forget_group, forget_user};

use crate::crypto::SecretBox;
use crate::settings::SettingsService;

/// Names the client gives its own entries (`flutter/lib/models/ab_model.dart:32-33`); a
/// shared book with one of them would replace that entry in the client's list.
pub const RESERVED_NAMES: [&str; 2] = ["My address book", "Legacy address book"];
/// Stored name of personal books (clients show their own label).
pub const PERSONAL_NAME: &str = RESERVED_NAMES[0];

/// An address book, without its contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Book {
    pub id: i64,
    /// The identifier clients use.
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

/// A shared book as one user sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub book: Book,
    /// Username of the owner; `None` when admin-managed.
    pub owner: Option<String>,
    /// The user's effective rule.
    pub rule: Rule,
}

/// A tag and its colour (ARGB, 32 bits, as the client sends it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub color: i64,
}

/// Input of [`AddressBookService::create_shared`].
#[derive(Clone, Debug)]
pub struct NewSharedBook {
    pub name: String,
    pub note: String,
    /// `None`: admin-managed.
    pub owner_user_id: Option<i64>,
}

/// Fields to change; `None` leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct SharedBookUpdate {
    pub name: Option<String>,
    pub note: Option<String>,
}

/// Address book operations. Holds no state of its own: cheap to build and to clone.
#[derive(Clone)]
pub struct AddressBookService {
    db: DatabaseConnection,
    settings: SettingsService,
    crypto: Arc<SecretBox>,
}

impl AddressBookService {
    /// `settings` supplies `addressbook.max_peers`; `crypto` seals shared-book passwords.
    pub fn new(db: DatabaseConnection, settings: SettingsService, crypto: Arc<SecretBox>) -> Self {
        Self {
            db,
            settings,
            crypto,
        }
    }

    /// The current `addressbook.max_peers` (clients' `max_peer_one_ab`).
    pub fn max_peers(&self) -> u32 {
        self.settings.snapshot().get(&ADDRESSBOOK_MAX_PEERS)
    }
}
