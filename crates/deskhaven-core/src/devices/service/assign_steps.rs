//! The transaction of [`DeviceService::assign`](super::DeviceService::assign), step by step
//! (see `assign` for the rules).

use std::sync::Arc;

use base64_022::Engine as _;
use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, Entity as Devices};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel,
    QueryFilter, QuerySelect,
};

use super::assign::{
    DeviceAssignment, book_error, book_view, invalid, not_found, placement, strategy_error,
};
use super::registry::same_uuid;
use super::{Inner, Requester};
use crate::addressbooks::{self, AddressBookError, AddressBookService};
use crate::console_audit::{self, NewConsoleAudit};
use crate::crypto::b64::KEY_B64;
use crate::devices::{DeviceError, preset_book, presets};
use crate::strategies::{self, Target};

/// The rows the assignment's names resolved to, held until the transaction ends.
#[derive(Default)]
struct Targets {
    owner: Option<i64>,
    group: Option<i64>,
    strategy: Option<i64>,
    /// The book's guid.
    book: Option<String>,
}

impl Inner {
    pub(super) fn address_books(&self) -> AddressBookService {
        AddressBookService::new(
            self.db.clone(),
            self.settings.clone(),
            Arc::clone(&self.crypto),
        )
    }

    /// Every write of an assignment and its audit row, in one transaction.
    pub(super) async fn assign_all(
        self: Arc<Self>,
        a: DeviceAssignment,
        by: Requester,
    ) -> Result<(), DeviceError> {
        let books = self.address_books();
        let now = self.now()?;
        let tx = begin_write(&self.db).await?;
        let row = lock_device(&tx, &a).await?;
        let found = resolve(&tx, &books, &a).await?;
        let held = strategies::assignment_in(&tx, Target::Device(row.id)).await?;
        let before = before_view(&tx, &books, &row, found.book.as_deref(), held).await?;
        let moves = found.owner.is_some_and(|o| row.owner_user_id != Some(o))
            || found.group.is_some_and(|g| row.device_group_id != Some(g));
        let stored = write_row(&tx, row, &a, &found, now).await?;
        if let Some(strategy_id) = found.strategy {
            let target = Target::Device(stored.id);
            strategies::assign_in(&tx, target, strategy_id)
                .await
                .map_err(strategy_error)?;
        }
        if let (Some(guid), Some(preset)) = (&found.book, &a.address_book) {
            place_entry(&tx, &books, guid, &a, preset)
                .await
                .map_err(book_error)?;
        }
        let after = placement(&stored, found.strategy.or(held));
        let entry = audit_entry(&a, by, before, after);
        console_audit::insert(&tx, &entry, now.timestamp_millis()).await?;
        tx.commit().await?;
        if moves || found.strategy.is_some() {
            self.strategies.cache().invalidate();
        }
        Ok(())
    }

    pub(super) fn now(&self) -> Result<TimestampMs, DbErr> {
        let ms = self.clock.now_ms();
        time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
    }
}

/// The device row, locked, if `a`'s id is registered with `a`'s uuid (always compared).
async fn lock_device(
    tx: &DatabaseTransaction,
    a: &DeviceAssignment,
) -> Result<device::Model, DeviceError> {
    let given = KEY_B64
        .decode(&a.uuid)
        .map_err(|_| DeviceError::UnknownDevice)?;
    let found = Devices::find()
        .filter(device::Column::RustdeskId.eq(&a.rustdesk_id))
        .lock_exclusive()
        .one(tx)
        .await?;
    found
        .filter(|row| same_uuid(&row.uuid, &given))
        .ok_or(DeviceError::UnknownDevice)
}

/// Every name of `a`, resolved inside `tx`.
async fn resolve(
    tx: &DatabaseTransaction,
    books: &AddressBookService,
    a: &DeviceAssignment,
) -> Result<Targets, DeviceError> {
    let mut found = Targets::default();
    if let Some(name) = &a.user_name {
        let owner = presets::find_user(tx, name).await?;
        found.owner = Some(owner.ok_or(not_found("user_name", "user"))?);
    }
    if let Some(name) = &a.device_group_name {
        let group = presets::find_device_group(tx, name).await?;
        found.group = Some(group.ok_or(not_found("device_group_name", "device group"))?);
    }
    if let Some(name) = &a.strategy_name {
        let strategy = strategies::id_by_name_in(tx, name).await?;
        found.strategy = Some(strategy.ok_or(not_found("strategy_name", "strategy"))?);
    }
    if let Some(preset) = &a.address_book {
        found.book = Some(book_guid(tx, books, &preset.name).await?);
    }
    Ok(found)
}

async fn book_guid(
    tx: &DatabaseTransaction,
    books: &AddressBookService,
    name: &str,
) -> Result<String, DeviceError> {
    match books.shared_by_name_in(tx, name).await {
        Ok(book) => Ok(book.guid),
        Err(AddressBookError::NotFound) => {
            Err(not_found("address_book_name", "shared address book"))
        }
        Err(AddressBookError::AmbiguousName) => Err(invalid(
            "address_book_name",
            "several shared address books have this name".to_owned(),
        )),
        Err(err) => Err(book_error(err)),
    }
}

/// Owner, device group, note and the administrator's names.
async fn write_row(
    tx: &DatabaseTransaction,
    row: device::Model,
    a: &DeviceAssignment,
    found: &Targets,
    now: TimestampMs,
) -> Result<device::Model, DbErr> {
    let mut active = row.into_active_model();
    if let Some(owner) = found.owner {
        active.owner_user_id = Set(Some(owner));
    }
    if let Some(group) = found.group {
        active.device_group_id = Set(Some(group));
    }
    let texts = [
        (&mut active.note, &a.note),
        (&mut active.display_name, &a.device_name),
        (&mut active.display_username, &a.device_username),
    ];
    for (column, value) in texts {
        if let Some(value) = value {
            *column = Set(Some(value.clone()));
        }
    }
    active.updated_at = Set(now);
    active.update(tx).await
}

async fn place_entry(
    tx: &DatabaseTransaction,
    books: &AddressBookService,
    guid: &str,
    a: &DeviceAssignment,
    preset: &presets::AddressBookPreset,
) -> Result<(), AddressBookError> {
    let names = (a.device_name.as_deref(), a.device_username.as_deref());
    let patch = preset_book::assign_patch(&a.rustdesk_id, preset, names.0, names.1)?;
    let tag = preset_book::new_tag(preset);
    let admin = addressbooks::Actor::Admin;
    books.place_entry_in(tx, admin, guid, patch, tag).await
}

/// The placement before the assignment, with the device's entry in the named book.
async fn before_view(
    tx: &DatabaseTransaction,
    books: &AddressBookService,
    row: &device::Model,
    guid: Option<&str>,
    held: Option<i64>,
) -> Result<serde_json::Value, DeviceError> {
    let mut before = placement(row, held);
    let Some(guid) = guid else {
        return Ok(before);
    };
    let admin = addressbooks::Actor::Admin;
    let entry = books.entry_view_in(tx, admin, guid, &row.rustdesk_id).await;
    let entry = entry
        .map_err(book_error)?
        .unwrap_or(serde_json::Value::Null);
    if let Some(before) = before.as_object_mut() {
        before.insert("address_book".to_owned(), entry);
    }
    Ok(before)
}

fn audit_entry(
    a: &DeviceAssignment,
    by: Requester,
    before: serde_json::Value,
    mut after: serde_json::Value,
) -> NewConsoleAudit {
    if let Some(after) = after.as_object_mut() {
        after.insert("address_book".to_owned(), book_view(a));
    }
    NewConsoleAudit {
        actor: by.actor,
        action: "device.assign".to_owned(),
        target_type: "device".to_owned(),
        target_id: a.rustdesk_id.clone(),
        before: Some(before),
        after: Some(after),
        ip: Some(by.client.addr),
    }
}
