//! Peers of a book: list (rule 1), add / update / delete (rule 2).

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::time;
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, SqlErr,
};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

use super::access::{Actor, authorize};
use super::error::{AddressBookError, invalid};
use super::peer::{Peer, PeerPatch, PeerSecret, check_extra};
use super::{AddressBookService, Rule};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

/// Ids deleted per statement (keeps far below SQLite's bound-parameter limit).
pub(super) const DELETE_CHUNK: usize = 500;

/// The sealed secret columns of a peer: the personal-book hash (a login credential: the
/// client sends it as the password) and the shared-book password.
#[derive(Clone, Copy)]
enum Secret {
    Hash,
    Password,
}

impl Secret {
    const fn column(self) -> &'static str {
        match self {
            Self::Hash => "hash_enc",
            Self::Password => "password_enc",
        }
    }
}

/// AAD context of a sealed peer secret (06 §5): `address_book_peers.<column>:<book
/// id>:<rustdesk id>`. The book id has no `:`, so the rest is the peer id, whatever it
/// contains.
fn secret_context(secret: Secret, book_id: i64, rustdesk_id: &str) -> String {
    format!(
        "address_book_peers.{}:{book_id}:{rustdesk_id}",
        secret.column()
    )
}

impl AddressBookService {
    /// One page of a book's peers, ordered by id; needs rule 1.
    pub async fn peers(
        &self,
        actor: Actor,
        guid: &str,
        page: PageRequest,
    ) -> Result<Page<Peer>, AddressBookError> {
        self.peers_matching(actor, guid, None, page).await
    }

    /// The peer `id` of the book; needs rule 1.
    pub async fn peer(&self, actor: Actor, guid: &str, id: &str) -> Result<Peer, AddressBookError> {
        let (book, _) = authorize(&self.db, actor, guid, Rule::ReadOnly, false).await?;
        let found = address_book_peer::Entity::find_by_id((book.id, id.to_owned()))
            .one(&self.db)
            .await?;
        let row = found.ok_or(AddressBookError::PeerNotFound)?;
        Ok(self.to_peer(&book, row))
    }

    /// [`peers`](Self::peers) narrowed to the peers whose id, alias, hostname, username or
    /// note contains `q` (case-insensitive, literal; ASCII only on SQLite).
    pub async fn peers_matching(
        &self,
        actor: Actor,
        guid: &str,
        q: Option<&str>,
        page: PageRequest,
    ) -> Result<Page<Peer>, AddressBookError> {
        let index = page.index().map_err(|p| invalid(p.field, p.reason))?;
        let (book, _) = authorize(&self.db, actor, guid, Rule::ReadOnly, false).await?;
        let mut select = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id));
        if let Some(needle) = q.filter(|q| !q.is_empty()) {
            select = select.filter(matching(&needle.to_lowercase()));
        }
        let paginator = select
            .order_by_asc(address_book_peer::Column::RustdeskId)
            .paginate(&self.db, page.page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(index).await?;
        Ok(Page {
            items: rows
                .into_iter()
                .map(|row| self.to_peer(&book, row))
                .collect(),
            total,
            page: page.page,
            page_size: page.page_size,
        })
    }

    pub(super) fn new_row(
        &self,
        book: &address_book::Model,
        patch: PeerPatch,
        now: TimestampMs,
    ) -> Result<address_book_peer::ActiveModel, AddressBookError> {
        let (hash_enc, password_enc) = match book.kind {
            Kind::Personal => (
                self.seal(Secret::Hash, book.id, &patch.id, patch.hash)?,
                None,
            ),
            Kind::Shared => {
                let sealed = self.seal(Secret::Password, book.id, &patch.id, patch.password)?;
                (None, sealed)
            }
        };
        Ok(address_book_peer::ActiveModel {
            address_book_id: Set(book.id),
            rustdesk_id: Set(patch.id),
            alias: Set(patch.alias.unwrap_or_default()),
            note: Set(patch.note.unwrap_or_default()),
            tags: Set(tags_json(patch.tags.unwrap_or_default())),
            hash_enc: Set(hash_enc),
            password_enc: Set(password_enc),
            username: Set(patch.username.unwrap_or_default()),
            hostname: Set(patch.hostname.unwrap_or_default()),
            platform: Set(patch.platform.unwrap_or_default()),
            force_always_relay: Set(patch.force_always_relay.unwrap_or(false)),
            rdp_port: Set(patch.rdp_port.unwrap_or_default()),
            rdp_username: Set(patch.rdp_username.unwrap_or_default()),
            login_name: Set(patch.login_name.unwrap_or_default()),
            extra: Set(Value::Object(patch.extra).to_string()),
            updated_at: Set(now),
        })
    }

    pub(super) fn changes(
        &self,
        book: &address_book::Model,
        current: address_book_peer::Model,
        patch: PeerPatch,
    ) -> Result<address_book_peer::ActiveModel, AddressBookError> {
        let mut extra = parse_extra(&current);
        extra.extend(patch.extra);
        check_extra(&extra)?;
        // A secret equal to the stored one is left sealed as it is: sealing again would
        // change the ciphertext (a fresh nonce) and so the row, for nothing.
        let sealed = |secret| match secret {
            Secret::Hash => current.hash_enc.as_deref(),
            Secret::Password => current.password_enc.as_deref(),
        };
        let same_hash = patch.hash.as_deref().is_some_and(|given| {
            self.open(Secret::Hash, &current, sealed(Secret::Hash))
                .as_str()
                == given
        });
        let same_password = patch.password.as_ref().is_some_and(|given| {
            let stored = self.open(Secret::Password, &current, sealed(Secret::Password));
            stored.as_str() == given.as_str()
        });
        let mut row: address_book_peer::ActiveModel = current.into();
        let texts = [
            (&mut row.alias, patch.alias),
            (&mut row.note, patch.note),
            (&mut row.username, patch.username),
            (&mut row.hostname, patch.hostname),
            (&mut row.platform, patch.platform),
            (&mut row.rdp_port, patch.rdp_port),
            (&mut row.rdp_username, patch.rdp_username),
            (&mut row.login_name, patch.login_name),
        ];
        for (column, value) in texts {
            if let Some(value) = value {
                *column = Set(value);
            }
        }
        if let Some(tags) = patch.tags {
            row.tags = Set(tags_json(tags));
        }
        if let Some(relay) = patch.force_always_relay {
            row.force_always_relay = Set(relay);
        }
        match book.kind {
            Kind::Personal if patch.hash.is_some() && !same_hash => {
                row.hash_enc = Set(self.seal(Secret::Hash, book.id, &patch.id, patch.hash)?);
            }
            Kind::Shared if patch.password.is_some() && !same_password => {
                let sealed = self.seal(Secret::Password, book.id, &patch.id, patch.password)?;
                row.password_enc = Set(sealed);
            }
            _ => {}
        }
        row.extra = Set(Value::Object(extra).to_string());
        row.updated_at = Set(time::now());
        Ok(row)
    }

    /// The sealed secret, or `None` for no or an empty one.
    fn seal(
        &self,
        secret: Secret,
        book_id: i64,
        rustdesk_id: &str,
        plain: Option<impl AsRef<str>>,
    ) -> Result<Option<String>, AddressBookError> {
        let Some(plain) = plain.filter(|p| !p.as_ref().is_empty()) else {
            return Ok(None);
        };
        let context = secret_context(secret, book_id, rustdesk_id);
        self.crypto
            .seal(&context, plain.as_ref().as_bytes())
            .map(Some)
            .map_err(AddressBookError::Crypto)
    }

    /// A stored row as clients read it. Unreadable stored values (corrupt JSON, a password
    /// sealed under a lost key) are logged and read as empty, so one bad row does not hide
    /// the whole book.
    pub(super) fn to_peer(
        &self,
        book: &address_book::Model,
        row: address_book_peer::Model,
    ) -> Peer {
        let secret = match book.kind {
            Kind::Personal => {
                let hash = self.open(Secret::Hash, &row, row.hash_enc.as_deref());
                PeerSecret::Hash(hash.as_str().to_owned())
            }
            Kind::Shared => {
                let sealed = row.password_enc.as_deref();
                PeerSecret::Password(self.open(Secret::Password, &row, sealed))
            }
        };
        let extra = parse_extra(&row);
        let tags = serde_json::from_str(&row.tags).unwrap_or_else(|err| {
            tracing::warn!(book = row.address_book_id, error = %err, "unreadable peer tags");
            Vec::new()
        });
        Peer {
            id: row.rustdesk_id,
            alias: row.alias,
            note: row.note,
            username: row.username,
            hostname: row.hostname,
            platform: row.platform,
            rdp_port: row.rdp_port,
            rdp_username: row.rdp_username,
            login_name: row.login_name,
            tags,
            force_always_relay: row.force_always_relay,
            secret,
            extra,
        }
    }

    fn open(
        &self,
        secret: Secret,
        row: &address_book_peer::Model,
        sealed: Option<&str>,
    ) -> Zeroizing<String> {
        let Some(sealed) = sealed else {
            return Zeroizing::default();
        };
        let context = secret_context(secret, row.address_book_id, &row.rustdesk_id);
        let opened = self
            .crypto
            .open(&context, sealed)
            .map(|bytes| Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned()));
        opened.unwrap_or_else(|err| {
            let column = secret.column();
            tracing::warn!(book = row.address_book_id, column, error = %err, "unreadable peer secret");
            Zeroizing::default()
        })
    }
}

/// Peers whose id, alias, hostname, username or note contains the lowercase `needle`.
fn matching(needle: &str) -> Condition {
    let columns = [
        address_book_peer::Column::RustdeskId,
        address_book_peer::Column::Alias,
        address_book_peer::Column::Hostname,
        address_book_peer::Column::Username,
        address_book_peer::Column::Note,
    ];
    columns.into_iter().fold(Condition::any(), |any, column| {
        let lowered = Expr::expr(Func::lower(Expr::col(column)));
        any.add(lowered.like(contains_pattern(needle)))
    })
}

fn tags_json(tags: Vec<String>) -> String {
    Value::from(tags).to_string()
}

pub(crate) fn parse_extra(row: &address_book_peer::Model) -> Map<String, Value> {
    serde_json::from_str(&row.extra).unwrap_or_else(|err| {
        tracing::warn!(book = row.address_book_id, error = %err, "unreadable peer extra");
        Map::new()
    })
}

/// The id was inserted concurrently (cannot happen while the book row is locked, kept as a
/// safety net).
pub(super) fn peer_conflict(err: sea_orm::DbErr) -> AddressBookError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => AddressBookError::PeerExists,
        _ => err.into(),
    }
}
