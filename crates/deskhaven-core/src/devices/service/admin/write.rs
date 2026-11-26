//! Each write runs in its own task and transaction with the device row locked, so the guard
//! decides on the committed state and the audit row commits with the change.

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
use crate::devices::strategy_keys;
use crate::strategies;
use crate::text::{self, NOTE_MAX_LEN};

/// `Some(None)` clears an optional reference.
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

    fn is_empty(&self) -> bool {
        !self.moves_device() && self.note.is_none() && self.status.is_none()
    }
}

impl DeviceService {
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
            if update.is_empty() {
                return Ok(DeviceRecord::from(current));
            }
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

    /// The audit row commits before the enqueue on purpose (an executed action must never lack its
    /// row), so a full queue leaves an audited attempt that queued nothing.
    pub async fn disconnect_guarded(
        &self,
        id: i64,
        conn_id: i32,
        guard: TxGuard<DeviceRecord>,
    ) -> Result<(), DeviceError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let row = device::Entity::find_by_id(id).one(&tx).await?;
            let record = DeviceRecord::from(row.ok_or(DeviceError::NotFound)?);
            guard.check(&tx, Some(&record), Some(&record)).await?;
            tx.commit().await?;
            inner
                .disconnects
                .disconnect(&record.rustdesk_id, &[conn_id])
                .map_err(|_| DeviceError::QueueFull)
        })
        .await
    }

    /// Its `device_strategy_keys` row stays, stamped with the deletion time.
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
            strategy_keys::stamp_deleted(&tx, &row.rustdesk_id).await?;
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

/// PostgreSQL puts them in its error text.
const FK_OWNER: &str = "fk_devices_owner_user";
const FK_GROUP: &str = "fk_devices_device_group";

/// A referenced row vanished between the check and the write (PostgreSQL only).
fn missing(err: DbErr, update: &DeviceUpdate) -> DeviceError {
    if !is_foreign_key_violation(&err) {
        return DeviceError::Database(err);
    }
    let text = err.to_string();
    let owner_set = matches!(update.owner_user_id, Some(Some(_)));
    let blame_owner = match (text.contains(FK_OWNER), text.contains(FK_GROUP)) {
        (true, _) => true,
        (false, true) => false,
        (false, false) => owner_set,
    };
    match blame_owner {
        true => unknown("owner_user_id", "no such user"),
        false => unknown("device_group_id", "no such device group"),
    }
}
