//! Tags of a book: list (rule 1), add / update / rename / delete (rule 2). Renaming and
//! deleting a tag also rewrites the tag lists of the book's peers.

use deskhaven_db::entity::address_book;
use deskhaven_db::entity::{address_book_peer, address_book_tag};
use deskhaven_db::{begin_write, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde_json::Value;

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::peer::check_tag_name;
use super::{AddressBookService, Rule, Tag};

/// A tag colour is a 32-bit ARGB value.
const MAX_COLOR: i64 = 0xFFFF_FFFF;
/// Most tags one book may hold.
pub const MAX_BOOK_TAGS: u64 = 1000;
const DELETE_CHUNK: usize = 500;

impl AddressBookService {
    /// Every tag of the book, ordered by name; needs rule 1.
    pub async fn tags(&self, actor: Actor, guid: &str) -> Result<Vec<Tag>, AddressBookError> {
        let (book, _) = authorize(&self.db, actor, guid, Rule::ReadOnly, false).await?;
        let rows = address_book_tag::Entity::find()
            .filter(address_book_tag::Column::AddressBookId.eq(book.id))
            .order_by_asc(address_book_tag::Column::Name)
            .all(&self.db)
            .await?;
        let tags = rows.into_iter().map(|row| Tag {
            name: row.name,
            color: row.color,
        });
        Ok(tags.collect())
    }

    /// Adds a tag; an existing tag of that name is kept as it is. Needs rule 2.
    pub async fn add_tag(
        &self,
        actor: Actor,
        guid: &str,
        tag: Tag,
    ) -> Result<(), AddressBookError> {
        check_tag(&tag)?;
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        if find_tag(&tx, book.id, &tag.name).await?.is_none() {
            ensure_room(&tx, book.id).await?;
            insert_tag(&tx, book.id, tag).await?;
            touch(&tx, &book).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Changes a tag's colour; needs rule 2.
    pub async fn update_tag(
        &self,
        actor: Actor,
        guid: &str,
        tag: Tag,
    ) -> Result<(), AddressBookError> {
        check_tag(&tag)?;
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let current = find_tag(&tx, book.id, &tag.name)
            .await?
            .ok_or(AddressBookError::TagNotFound)?;
        let mut changes: address_book_tag::ActiveModel = current.into();
        changes.color = Set(tag.color);
        changes.update(&tx).await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Renames tag `old` to `new` (keeping its colour), in the tag list and in every peer
    /// of the book; needs rule 2.
    pub async fn rename_tag(
        &self,
        actor: Actor,
        guid: &str,
        old: &str,
        new: &str,
    ) -> Result<(), AddressBookError> {
        check_tag_name("new", new)?;
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let current = find_tag(&tx, book.id, old)
            .await?
            .ok_or(AddressBookError::TagNotFound)?;
        if old == new {
            return Ok(());
        }
        if find_tag(&tx, book.id, new).await?.is_some() {
            return Err(AddressBookError::TagExists);
        }
        let color = current.color;
        address_book_tag::Entity::delete_by_id((book.id, old.to_owned()))
            .exec(&tx)
            .await?;
        let renamed = Tag {
            name: new.to_owned(),
            color,
        };
        insert_tag(&tx, book.id, renamed).await?;
        rewrite_peer_tags(&tx, &book, |tag| match tag == old {
            true => Some(new.to_owned()),
            false => Some(tag.to_owned()),
        })
        .await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Deletes these tags from the book and from its peers; unknown names are ignored.
    /// Needs rule 2.
    pub async fn delete_tags(
        &self,
        actor: Actor,
        guid: &str,
        names: &[String],
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        for chunk in names.chunks(DELETE_CHUNK) {
            address_book_tag::Entity::delete_many()
                .filter(address_book_tag::Column::AddressBookId.eq(book.id))
                .filter(address_book_tag::Column::Name.is_in(chunk.iter().cloned()))
                .exec(&tx)
                .await?;
        }
        rewrite_peer_tags(&tx, &book, |tag| {
            (!names.iter().any(|n| n == tag)).then(|| tag.to_owned())
        })
        .await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }
}

fn check_tag(tag: &Tag) -> Result<(), AddressBookError> {
    check_tag_name("name", &tag.name)?;
    if !(0..=MAX_COLOR).contains(&tag.color) {
        return Err(invalid("color", "must be a 32-bit ARGB value"));
    }
    Ok(())
}

async fn find_tag(
    tx: &DatabaseTransaction,
    book_id: i64,
    name: &str,
) -> Result<Option<address_book_tag::Model>, AddressBookError> {
    let found = address_book_tag::Entity::find_by_id((book_id, name.to_owned()))
        .one(tx)
        .await?;
    Ok(found)
}

async fn ensure_room(tx: &DatabaseTransaction, book_id: i64) -> Result<(), AddressBookError> {
    let count = address_book_tag::Entity::find()
        .filter(address_book_tag::Column::AddressBookId.eq(book_id))
        .count(tx)
        .await?;
    match count >= MAX_BOOK_TAGS {
        true => Err(invalid(
            "name",
            format!("an address book holds at most {MAX_BOOK_TAGS} tags"),
        )),
        false => Ok(()),
    }
}

async fn insert_tag(
    tx: &DatabaseTransaction,
    book_id: i64,
    tag: Tag,
) -> Result<(), AddressBookError> {
    let row = address_book_tag::ActiveModel {
        address_book_id: Set(book_id),
        name: Set(tag.name),
        color: Set(tag.color),
    };
    row.insert(tx).await?;
    Ok(())
}

/// Maps every tag of every peer through `map` (`None` drops it, duplicates collapse) and
/// writes back the peers whose list changed.
async fn rewrite_peer_tags(
    tx: &DatabaseTransaction,
    book: &address_book::Model,
    map: impl Fn(&str) -> Option<String>,
) -> Result<(), AddressBookError> {
    let peers = address_book_peer::Entity::find()
        .select_only()
        .column(address_book_peer::Column::RustdeskId)
        .column(address_book_peer::Column::Tags)
        .filter(address_book_peer::Column::AddressBookId.eq(book.id))
        .into_tuple::<(String, String)>()
        .all(tx)
        .await?;
    let now = time::now();
    for (rustdesk_id, stored) in peers {
        let Ok(tags) = serde_json::from_str::<Vec<String>>(&stored) else {
            continue;
        };
        let mut mapped: Vec<String> = Vec::with_capacity(tags.len());
        for tag in tags.iter().filter_map(|t| map(t)) {
            if !mapped.contains(&tag) {
                mapped.push(tag);
            }
        }
        if mapped == tags {
            continue;
        }
        let changes = address_book_peer::ActiveModel {
            tags: Set(Value::from(mapped).to_string()),
            updated_at: Set(now),
            ..Default::default()
        };
        address_book_peer::Entity::update_many()
            .set(changes)
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .filter(address_book_peer::Column::RustdeskId.eq(rustdesk_id))
            .exec(tx)
            .await?;
    }
    Ok(())
}
