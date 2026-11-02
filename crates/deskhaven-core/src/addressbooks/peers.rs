//! Peers of a book: list (rule 1), add / update / delete (rule 2).

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::time::TimestampMs;
use deskhaven_db::{begin_write, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, SqlErr,
};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::peer::{Peer, PeerPatch, PeerSecret, check_extra};
use super::{AddressBookService, Rule};
use crate::pagination::{Page, PageRequest};

/// Ids deleted per statement (keeps far below SQLite's bound-parameter limit).
const DELETE_CHUNK: usize = 500;

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
        let index = page.index().map_err(|p| invalid(p.field, p.reason))?;
        let (book, _) = authorize(&self.db, actor, guid, Rule::ReadOnly, false).await?;
        let paginator = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
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

    /// Adds a peer; needs rule 2. Refused when the id is already in the book
    /// ([`AddressBookError::PeerExists`]) or the book holds `addressbook.max_peers` peers.
    pub async fn add_peer(
        &self,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
    ) -> Result<(), AddressBookError> {
        let max = self.max_peers();
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let existing = address_book_peer::Entity::find_by_id((book.id, patch.id.clone()))
            .one(&tx)
            .await?;
        if existing.is_some() {
            return Err(AddressBookError::PeerExists);
        }
        let count = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .count(&tx)
            .await?;
        if count >= u64::from(max) {
            return Err(AddressBookError::Full { max });
        }
        let row = self.new_row(&book, patch, time::now())?;
        row.insert(&tx).await.map_err(peer_conflict)?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Changes the fields `patch` carries (unknown fields are merged into `extra`); needs
    /// rule 2.
    pub async fn update_peer(
        &self,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let current = address_book_peer::Entity::find_by_id((book.id, patch.id.clone()))
            .one(&tx)
            .await?
            .ok_or(AddressBookError::PeerNotFound)?;
        let changes = self.changes(&book, current, patch)?;
        changes.update(&tx).await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Removes the peers with these ids; ids not in the book are ignored. At most
    /// `addressbook.max_peers` ids (a book holds no more), else nothing is touched. Needs
    /// rule 2.
    pub async fn delete_peers(
        &self,
        actor: Actor,
        guid: &str,
        ids: &[String],
    ) -> Result<(), AddressBookError> {
        let max = self.max_peers();
        if ids.len() as u64 > u64::from(max) {
            return Err(invalid("ids", format!("at most {max} per request")));
        }
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        for chunk in ids.chunks(DELETE_CHUNK) {
            address_book_peer::Entity::delete_many()
                .filter(address_book_peer::Column::AddressBookId.eq(book.id))
                .filter(address_book_peer::Column::RustdeskId.is_in(chunk.iter().cloned()))
                .exec(&tx)
                .await?;
        }
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    fn new_row(
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

    fn changes(
        &self,
        book: &address_book::Model,
        current: address_book_peer::Model,
        patch: PeerPatch,
    ) -> Result<address_book_peer::ActiveModel, AddressBookError> {
        let mut extra = parse_extra(&current);
        extra.extend(patch.extra);
        check_extra(&extra)?;
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
            Kind::Personal if patch.hash.is_some() => {
                row.hash_enc = Set(self.seal(Secret::Hash, book.id, &patch.id, patch.hash)?);
            }
            Kind::Shared if patch.password.is_some() => {
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
    fn to_peer(&self, book: &address_book::Model, row: address_book_peer::Model) -> Peer {
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
fn peer_conflict(err: sea_orm::DbErr) -> AddressBookError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => AddressBookError::PeerExists,
        _ => err.into(),
    }
}
