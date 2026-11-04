//! Peer writes: add, update and delete (rule 2). Each `*_guarded` variant runs a
//! [`TxGuard`] inside its write transaction (the console's audit row); the plain variants
//! are the same writes without one.

use deskhaven_db::entity::address_book_peer;
use deskhaven_db::{begin_write, time};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::Value;

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::peer::PeerPatch;
use super::peers::{DELETE_CHUNK, peer_conflict};
use super::{AddressBookService, Rule, snapshot};
use crate::console_audit::TxGuard;

impl AddressBookService {
    /// Adds a peer; needs rule 2. Refused when the id is already in the book
    /// ([`AddressBookError::PeerExists`]) or the book holds `addressbook.max_peers` peers.
    pub async fn add_peer(
        &self,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
    ) -> Result<(), AddressBookError> {
        self.add_peer_guarded(actor, guid, patch, TxGuard::none())
            .await
    }

    /// [`add_peer`](Self::add_peer), with `guard` deciding on the new peer.
    pub async fn add_peer_guarded(
        &self,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
        guard: TxGuard<Value>,
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
        let created = row.insert(&tx).await.map_err(peer_conflict)?;
        touch(&tx, &book).await?;
        guard
            .check(&tx, None, Some(&snapshot::peer(&created)))
            .await?;
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
        self.update_peer_guarded(actor, guid, patch, TxGuard::none())
            .await
    }

    /// [`update_peer`](Self::update_peer), with `guard` deciding on the peer before and
    /// after.
    pub async fn update_peer_guarded(
        &self,
        actor: Actor,
        guid: &str,
        patch: PeerPatch,
        guard: TxGuard<Value>,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let current = address_book_peer::Entity::find_by_id((book.id, patch.id.clone()))
            .one(&tx)
            .await?
            .ok_or(AddressBookError::PeerNotFound)?;
        let before = snapshot::peer(&current);
        let changes = self.changes(&book, current, patch)?;
        let updated = changes.update(&tx).await?;
        touch(&tx, &book).await?;
        let after = snapshot::peer(&updated);
        guard.check(&tx, Some(&before), Some(&after)).await?;
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
        self.delete_peers_guarded(actor, guid, ids, TxGuard::none())
            .await
    }

    /// [`delete_peers`](Self::delete_peers), with `guard` deciding on the removed peers
    /// (no `after`). When none of the ids is in the book nothing changes and the guard is
    /// not consulted.
    pub async fn delete_peers_guarded(
        &self,
        actor: Actor,
        guid: &str,
        ids: &[String],
        guard: TxGuard<Value>,
    ) -> Result<(), AddressBookError> {
        let max = self.max_peers();
        if ids.len() as u64 > u64::from(max) {
            return Err(invalid("ids", format!("at most {max} per request")));
        }
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let mut removed = Vec::new();
        for chunk in ids.chunks(DELETE_CHUNK) {
            let in_chunk = address_book_peer::Column::RustdeskId.is_in(chunk.iter().cloned());
            let rows = address_book_peer::Entity::find()
                .filter(address_book_peer::Column::AddressBookId.eq(book.id))
                .filter(in_chunk.clone())
                .all(&tx)
                .await?;
            address_book_peer::Entity::delete_many()
                .filter(address_book_peer::Column::AddressBookId.eq(book.id))
                .filter(in_chunk)
                .exec(&tx)
                .await?;
            removed.extend(rows);
        }
        touch(&tx, &book).await?;
        if !removed.is_empty() {
            let ids = removed.iter().map(|row| row.rustdesk_id.as_str());
            guard
                .check(&tx, Some(&snapshot::peer_ids(ids)), None)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
