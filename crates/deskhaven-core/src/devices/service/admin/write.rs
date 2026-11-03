//! The console's device writes. Both run in their own task and write transaction with the
//! device row locked, so the guard decides on the committed state and the audit row commits
//! with the change. Moving a device (owner, group) invalidates the strategy cache after the
//! commit, like the registry's own moves (see `heartbeat`).

use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::entity::{device_group, user};
use deskhaven_db::{is_foreign_key_violation, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DbErr, EntityTrait, QuerySelect};

use super::DeviceRecord;
use crate::console_audit::TxGuard;
use crate::devices::DeviceError;
use crate::devices::service::DeviceService;
use crate::strategies;
use crate::text::{self, NOTE_MAX_LEN};

/// Fields to change; `None` leaves a field as it is. For the optional references,
/// `Some(None)` clears them.
#[derive(Clone, Debug, Default)]
pub struct DeviceUpdate {
    pub owner_user_id: Option<Option<i64>>,
    pub device_group_id: Option<Option<i64>>,
    /// Blank clears the note.
    pub note: Option<String>,
    pub status: Option<DeviceStatus>,
}

impl DeviceUpdate {
    fn moves_device(&self) -> bool {
        self.owner_user_id.is_some() || self.device_group_id.is_some()
    }
}

impl DeviceService {
    /// Applies `update` to device `id`, with `guard` deciding on the device before and after.
    pub async fn update_guarded(
        &self,
        id: i64,
        update: DeviceUpdate,
        guard: TxGuard<DeviceRecord>,
    ) -> Result<DeviceRecord, DeviceError> {
        if let Some(note) = &update.note
            && let Some(reason) = text::problem(note, NOTE_MAX_LEN, true)
        {
            return Err(DeviceError::Invalid {
                field: "note",
                reason,
            });
        }
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = device::Entity::find_by_id(id)
                .lock_exclusive()
                .one(&tx)
                .await?
                .ok_or(DeviceError::NotFound)?;
            check_references(&tx, &update).await?;
            let before = DeviceRecord::from(current.clone());
            let mut changes: device::ActiveModel = current.into();
            if let Some(owner) = update.owner_user_id {
                changes.owner_user_id = Set(owner);
            }
            if let Some(group) = update.device_group_id {
                changes.device_group_id = Set(group);
            }
            if let Some(note) = &update.note {
                changes.note = Set(Some(note.clone()).filter(|n| !n.trim().is_empty()));
            }
            if let Some(status) = update.status {
                changes.status = Set(status);
            }
            changes.updated_at = Set(time::now());
            let updated =
                DeviceRecord::from(changes.update(&tx).await.map_err(|e| missing(e, &update))?);
            guard.check(&tx, Some(&before), Some(&updated)).await?;
            tx.commit().await?;
            if update.moves_device() {
                inner.strategies.cache().invalidate();
            }
            Ok(updated)
        })
        .await
    }

    /// Deletes device `id` and its strategy assignment, with `guard` deciding on the device
    /// (no `after`); `false` if it did not exist. Then runs the deleted hooks.
    pub async fn delete_guarded(
        &self,
        id: i64,
        guard: TxGuard<DeviceRecord>,
    ) -> Result<bool, DeviceError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let found = device::Entity::find_by_id(id)
                .lock_exclusive()
                .one(&tx)
                .await?;
            let Some(row) = found else {
                return Ok(false);
            };
            guard
                .check(&tx, Some(&DeviceRecord::from(row.clone())), None)
                .await?;
            strategies::forget_device(&tx, row.id).await?;
            device::Entity::delete_by_id(row.id).exec(&tx).await?;
            tx.commit().await?;
            inner.strategies.cache().invalidate();
            inner.presence.forget(&row.rustdesk_id);
            inner.sysinfo_writes.forget(&row.rustdesk_id);
            inner.run_deleted_hooks(&row.rustdesk_id);
            Ok(true)
        })
        .await
    }
}

/// The owner and device group `update` names exist.
async fn check_references(
    tx: &sea_orm::DatabaseTransaction,
    update: &DeviceUpdate,
) -> Result<(), DeviceError> {
    if let Some(Some(owner)) = update.owner_user_id
        && user::Entity::find_by_id(owner).one(tx).await?.is_none()
    {
        return Err(unknown("owner_user_id", "no such user"));
    }
    if let Some(Some(group)) = update.device_group_id
        && device_group::Entity::find_by_id(group)
            .one(tx)
            .await?
            .is_none()
    {
        return Err(unknown("device_group_id", "no such device group"));
    }
    Ok(())
}

fn unknown(field: &'static str, reason: &str) -> DeviceError {
    DeviceError::Invalid {
        field,
        reason: reason.to_owned(),
    }
}

/// A referenced row vanished between the check and the write (PostgreSQL; SQLite's write
/// transaction excludes that).
fn missing(err: DbErr, update: &DeviceUpdate) -> DeviceError {
    if !is_foreign_key_violation(&err) {
        return DeviceError::Database(err);
    }
    match update.owner_user_id {
        Some(Some(_)) => unknown("owner_user_id", "no such user"),
        _ => unknown("device_group_id", "no such device group"),
    }
}
