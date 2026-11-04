//! Writes of the shared book itself: create, rename / note, delete. Each `*_guarded`
//! variant runs a [`TxGuard`] inside its write transaction (the console's audit row); the
//! plain variants are the same writes without one.

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::{begin_write, is_foreign_key_violation, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ModelTrait};
use serde_json::Value;

use super::access::{Actor, authorize, is_steward};
use super::books::{ensure_name_free, new_guid, validate_name, validate_note};
use super::error::AddressBookError;
use super::{AddressBookService, Book, NewSharedBook, Rule, SharedBookUpdate, snapshot};
use crate::console_audit::TxGuard;

impl AddressBookService {
    /// Creates a shared book (an administrator's operation: no actor check).
    pub async fn create_shared(&self, new: NewSharedBook) -> Result<Book, AddressBookError> {
        self.create_shared_guarded(new, TxGuard::none()).await
    }

    /// [`create_shared`](Self::create_shared), with `guard` deciding on the new book.
    pub async fn create_shared_guarded(
        &self,
        new: NewSharedBook,
        guard: TxGuard<Value>,
    ) -> Result<Book, AddressBookError> {
        let name = validate_name(&new.name)?;
        let note = validate_note(&new.note)?;
        let tx = begin_write(&self.db).await?;
        ensure_name_free(&tx, &name, None).await?;
        let now = time::now();
        let row = address_book::ActiveModel {
            guid: Set(new_guid()),
            kind: Set(Kind::Shared),
            owner_user_id: Set(new.owner_user_id),
            name: Set(name),
            note: Set(note),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let created =
            row.insert(&tx)
                .await
                .map_err(|err| match is_foreign_key_violation(&err) {
                    true => AddressBookError::UserNotFound,
                    false => err.into(),
                })?;
        guard
            .check(&tx, None, Some(&snapshot::book(&created)))
            .await?;
        tx.commit().await?;
        Ok(created.into())
    }

    /// Renames a shared book or changes its note; needs full control.
    pub async fn update_shared(
        &self,
        actor: Actor,
        guid: &str,
        update: SharedBookUpdate,
    ) -> Result<Book, AddressBookError> {
        self.update_shared_guarded(actor, guid, update, TxGuard::none())
            .await
    }

    /// [`update_shared`](Self::update_shared), with `guard` deciding on the book before and
    /// after. An update that changes nothing writes nothing and does not consult the guard.
    pub async fn update_shared_guarded(
        &self,
        actor: Actor,
        guid: &str,
        update: SharedBookUpdate,
        guard: TxGuard<Value>,
    ) -> Result<Book, AddressBookError> {
        let name = update.name.as_deref().map(validate_name).transpose()?;
        let note = update.note.as_deref().map(validate_note).transpose()?;
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        let name = name.filter(|n| *n != book.name);
        let note = note.filter(|n| *n != book.note);
        if name.is_none() && note.is_none() {
            return Ok(book.into());
        }
        let mut changes: address_book::ActiveModel = book.clone().into();
        if let Some(name) = name {
            ensure_name_free(&tx, &name, Some(book.id)).await?;
            changes.name = Set(name);
        }
        if let Some(note) = note {
            changes.note = Set(note);
        }
        changes.updated_at = Set(time::now());
        let updated = changes.update(&tx).await?;
        let (before, after) = (snapshot::book(&book), snapshot::book(&updated));
        guard.check(&tx, Some(&before), Some(&after)).await?;
        tx.commit().await?;
        Ok(updated.into())
    }

    /// Deletes a shared book with its peers, tags and shares; needs full control, and is
    /// the owner's or the console's (see [`is_steward`]).
    pub async fn delete_shared(&self, actor: Actor, guid: &str) -> Result<(), AddressBookError> {
        self.delete_shared_guarded(actor, guid, TxGuard::none())
            .await
    }

    /// [`delete_shared`](Self::delete_shared), with `guard` deciding on the book (no `after`).
    pub async fn delete_shared_guarded(
        &self,
        actor: Actor,
        guid: &str,
        guard: TxGuard<Value>,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        if !is_steward(actor, &book) {
            return Err(AddressBookError::Forbidden);
        }
        guard.check(&tx, Some(&snapshot::book(&book)), None).await?;
        book.delete(&tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
