//! Whole-book operations: read everything a book holds, replace it in one transaction.
//! Legacy clients (`/api/ab`, 03 §5.2) keep the whole book in memory and push it back whole;
//! the per-item operations of [`peers`](super::peers) and [`tags`](super::tags) serve the
//! current ones. Both act on the same rows.
//!
//! Replacing is a merge by peer id, not a delete-and-insert: a pushed peer updates the
//! stored one with the fields it carries ([`PeerPatch`] semantics), so what a legacy client
//! never sends (`note`, `forceAlwaysRelay`, the RDP fields, unknown fields kept in `extra`)
//! survives its pushes. Peers that are not pushed are deleted, tags are replaced.

use std::collections::{HashMap, HashSet};

use deskhaven_db::begin_write;
use deskhaven_db::entity::{address_book_peer, address_book_tag};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter, QueryOrder,
};

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::peer::{Peer, PeerPatch};
use super::tags::{MAX_BOOK_TAGS, check_tag};
use super::{AddressBookService, Rule, Tag};

/// Colour of a tag nobody gave a colour (ARGB `0xFF0071FF`, the client's accent blue,
/// `MyTheme.accent` in `flutter/lib/common.dart`).
pub const DEFAULT_TAG_COLOR: i64 = 0xFF00_71FF;
/// Rows per insert statement (keeps far below SQLite's bound-parameter limit).
const WRITE_CHUNK: usize = 200;
/// Ids deleted per statement.
const DELETE_CHUNK: usize = 500;

/// Everything a book holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookContents {
    /// Ordered by id.
    pub peers: Vec<Peer>,
    /// Ordered by name.
    pub tags: Vec<Tag>,
    pub updated_at: TimestampMs,
}

/// A tag of a [`BookReplacement`]; `color: None` keeps the stored colour of that name, or
/// takes [`DEFAULT_TAG_COLOR`] for a new tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacementTag {
    pub name: String,
    pub color: Option<i64>,
}

/// What a book holds after [`AddressBookService::replace_contents`].
#[derive(Clone, Debug, Default)]
pub struct BookReplacement {
    /// A repeated id counts once, its first occurrence wins.
    pub peers: Vec<PeerPatch>,
    /// A repeated name counts once, its first occurrence wins.
    pub tags: Vec<ReplacementTag>,
}

impl AddressBookService {
    /// Every peer and tag of the book; needs rule 1.
    pub async fn contents(
        &self,
        actor: Actor,
        guid: &str,
    ) -> Result<BookContents, AddressBookError> {
        let (book, _) = authorize(&self.db, actor, guid, Rule::ReadOnly, false).await?;
        let peers = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .order_by_asc(address_book_peer::Column::RustdeskId)
            .all(&self.db)
            .await?;
        let tags = address_book_tag::Entity::find()
            .filter(address_book_tag::Column::AddressBookId.eq(book.id))
            .order_by_asc(address_book_tag::Column::Name)
            .all(&self.db)
            .await?;
        Ok(BookContents {
            peers: peers
                .into_iter()
                .map(|row| self.to_peer(&book, row))
                .collect(),
            tags: tags
                .into_iter()
                .map(|row| Tag {
                    name: row.name,
                    color: row.color,
                })
                .collect(),
            updated_at: book.updated_at,
        })
    }

    /// Makes the book hold exactly `new` (see the module docs), in one transaction: any
    /// failure leaves the book as it was. Needs rule 2. Refused before anything is touched
    /// when `new` holds more distinct peers than `addressbook.max_peers`, more than
    /// [`MAX_BOOK_TAGS`] distinct tags, or an invalid tag.
    pub async fn replace_contents(
        &self,
        actor: Actor,
        guid: &str,
        new: BookReplacement,
    ) -> Result<(), AddressBookError> {
        let max = self.max_peers();
        let peers = distinct(new.peers, |patch| patch.id.as_str());
        if peers.len() as u64 > u64::from(max) {
            return Err(AddressBookError::Full { max });
        }
        let tags = distinct(new.tags, |tag| tag.name.as_str());
        if tags.len() as u64 > MAX_BOOK_TAGS {
            return Err(invalid(
                "tags",
                format!("an address book holds at most {MAX_BOOK_TAGS} tags"),
            ));
        }
        for tag in &tags {
            check_tag(&Tag {
                name: tag.name.clone(),
                color: tag.color.unwrap_or(DEFAULT_TAG_COLOR),
            })?;
        }
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let stored = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .all(&tx)
            .await?;
        let mut stored: HashMap<String, address_book_peer::Model> = stored
            .into_iter()
            .map(|row| (row.rustdesk_id.clone(), row))
            .collect();
        let pushed: HashSet<&str> = peers.iter().map(|patch| patch.id.as_str()).collect();
        let gone: Vec<String> = stored
            .keys()
            .filter(|id| !pushed.contains(id.as_str()))
            .cloned()
            .collect();
        for chunk in gone.chunks(DELETE_CHUNK) {
            address_book_peer::Entity::delete_many()
                .filter(address_book_peer::Column::AddressBookId.eq(book.id))
                .filter(address_book_peer::Column::RustdeskId.is_in(chunk.iter().cloned()))
                .exec(&tx)
                .await?;
        }
        let now = time::now();
        let mut added = Vec::new();
        for patch in peers {
            match stored.remove(&patch.id) {
                Some(current) => {
                    self.changes(&book, current, patch)?.update(&tx).await?;
                }
                None => added.push(self.new_row(&book, patch, now)?),
            }
        }
        for chunk in chunks(added) {
            address_book_peer::Entity::insert_many(chunk)
                .exec(&tx)
                .await?;
        }
        let colors = stored_colors(&tx, book.id).await?;
        address_book_tag::Entity::delete_many()
            .filter(address_book_tag::Column::AddressBookId.eq(book.id))
            .exec(&tx)
            .await?;
        let rows = tags.into_iter().map(|tag| address_book_tag::ActiveModel {
            address_book_id: Set(book.id),
            color: Set(tag
                .color
                .or_else(|| colors.get(&tag.name).copied())
                .unwrap_or(DEFAULT_TAG_COLOR)),
            name: Set(tag.name),
        });
        for chunk in chunks(rows.collect()) {
            address_book_tag::Entity::insert_many(chunk)
                .exec(&tx)
                .await?;
        }
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }
}

/// `items` without repeats of `key`, first occurrence kept.
fn distinct<T>(items: Vec<T>, key: impl Fn(&T) -> &str) -> Vec<T> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(key(item).to_owned()))
        .collect()
}

fn chunks<T>(mut items: Vec<T>) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    while items.len() > WRITE_CHUNK {
        let rest = items.split_off(WRITE_CHUNK);
        out.push(std::mem::replace(&mut items, rest));
    }
    if !items.is_empty() {
        out.push(items);
    }
    out
}

async fn stored_colors(
    tx: &DatabaseTransaction,
    book_id: i64,
) -> Result<HashMap<String, i64>, AddressBookError> {
    let rows = address_book_tag::Entity::find()
        .filter(address_book_tag::Column::AddressBookId.eq(book_id))
        .all(tx)
        .await?;
    Ok(rows.into_iter().map(|row| (row.name, row.color)).collect())
}
