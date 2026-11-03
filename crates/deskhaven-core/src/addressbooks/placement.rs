//! A device's entry in a shared book written inside the caller's write transaction, so it
//! commits or rolls back with the rest of a placement (`--assign`, M2-19). Rule 2 is
//! checked as for any write.
//!
//! A missing entry is added (refused at `addressbook.max_peers`); an existing one takes the
//! fields `patch` carries, and its tags gain the patch's (never lose any). A tag the book
//! lacks is created first (refused at the book's tag limit).

use deskhaven_db::entity::address_book_peer;
use deskhaven_db::time;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
};

use super::access::{Actor, authorize};
use super::books::{shared_by_name, touch};
use super::error::AddressBookError;
use super::peer::PeerPatch;
use super::peers::peer_conflict;
use super::tags::{check_tag, ensure_room, find_tag, insert_tag};
use super::{AddressBookService, Book, Rule, Tag};

impl AddressBookService {
    /// [`find_shared_by_name`](Self::find_shared_by_name) inside `tx`.
    pub(crate) async fn shared_by_name_in(
        &self,
        tx: &DatabaseTransaction,
        name: &str,
    ) -> Result<Book, AddressBookError> {
        shared_by_name(tx, name).await
    }

    /// Adds or updates `patch`'s entry in book `guid` as `actor`, inside `tx` (see the module
    /// documentation); creates `tag` in the book if needed. `patch.tags` lists the tags to add.
    pub(crate) async fn place_entry_in(
        &self,
        tx: &DatabaseTransaction,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
        tag: Option<Tag>,
    ) -> Result<(), AddressBookError> {
        let (book, _) = authorize(tx, actor, guid, Rule::ReadWrite, true).await?;
        if let Some(tag) = tag {
            check_tag(&tag)?;
            if find_tag(tx, book.id, &tag.name).await?.is_none() {
                ensure_room(tx, book.id).await?;
                insert_tag(tx, book.id, tag).await?;
            }
        }
        let existing = address_book_peer::Entity::find_by_id((book.id, patch.id.clone()))
            .one(tx)
            .await?;
        match existing {
            Some(current) => {
                let tags = merged_tags(&current.tags, patch.tags.clone());
                let patch = PeerPatch { tags, ..patch };
                self.changes(&book, current, patch)?.update(tx).await?;
            }
            None => {
                self.ensure_peer_room(tx, book.id).await?;
                let row = self.new_row(&book, patch, time::now())?;
                row.insert(tx).await.map_err(peer_conflict)?;
            }
        }
        touch(tx, &book).await
    }

    async fn ensure_peer_room(
        &self,
        tx: &DatabaseTransaction,
        book_id: i64,
    ) -> Result<(), AddressBookError> {
        let max = self.max_peers();
        let count = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book_id))
            .count(tx)
            .await?;
        match count >= u64::from(max) {
            true => Err(AddressBookError::Full { max }),
            false => Ok(()),
        }
    }
}

/// The stored tags plus `added` (in order, without duplicates); `None` when nothing is added.
fn merged_tags(stored: &str, added: Option<Vec<String>>) -> Option<Vec<String>> {
    let added = added?;
    let mut tags: Vec<String> = serde_json::from_str(stored).unwrap_or_default();
    for tag in added {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    Some(tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_tags_are_appended_once_and_none_adds_nothing() {
        let stored = r#"["printers","lobby"]"#;

        let merged = merged_tags(stored, Some(vec!["lobby".to_owned(), "new".to_owned()]));

        assert_eq!(
            merged,
            Some(vec!["printers".into(), "lobby".into(), "new".into()])
        );
        assert_eq!(merged_tags(stored, None), None);
    }
}
