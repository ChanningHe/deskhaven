//! [`DeviceService`]: what the device endpoints and the console need of the registry beyond
//! the signal server's registrations ([`DeviceRepository`](super::DeviceRepository)).
//!
//! **Identity** (06 §2): device endpoints carry no credential, only `id` and `uuid` (base64 of
//! the raw uuid the device sent in RegisterPk). The id must be a registered device, and with
//! `devices.verify_uuid` (default) the uuid must equal the stored one (constant-time
//! comparison). Both failures look the same to the caller.
//!
//! **Strategy cache**: deleting a device or device group removes its strategy assignment in
//! the same transaction ([`strategies::forget_device`] / [`strategies::forget_device_group`])
//! and invalidates the cache after commit; so does any change of a device's owner or device
//! group made here (presets). Writes run in their own task, so a dropped request can neither
//! interrupt a commit nor lose the invalidation that follows it.

use std::future::Future;
use std::sync::Arc;

use deskhaven_config::settings::{AcceptPresets, DEVICES_VERIFY_UUID};
use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, Entity as Devices};
use deskhaven_db::entity::device_group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect, SqlErr,
};
use subtle::ConstantTimeEq;

use super::DeviceError;
use super::presence::{Identity, Presence};
use super::preset_book;
use super::presets::{self, Presets, RowPresets};
use super::sysinfo::Sysinfo;
use crate::addressbooks::AddressBookService;
use crate::crypto::{SecretBox, b64::KEY_B64};
use crate::settings::SettingsService;
use crate::strategies::{self, StrategyService};
use crate::text::{self, NOTE_MAX_LEN};
use base64_022::Engine as _;

/// Longest device group name, in characters.
pub const DEVICE_GROUP_NAME_MAX_LEN: usize = 64;

/// A `POST /api/sysinfo` body, parsed.
#[derive(Clone, Debug, Default)]
pub struct SysinfoReport {
    pub rustdesk_id: String,
    /// As sent: base64 of the device uuid.
    pub uuid: String,
    pub sysinfo: Sysinfo,
    pub presets: Presets,
}

/// Result of [`DeviceService::report_sysinfo`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysinfoOutcome {
    /// Written (first report, or the content changed).
    Stored,
    /// Same content as stored: nothing written.
    Unchanged,
    /// Unknown id, or a uuid that does not match.
    NotFound,
}

/// Device registry operations. Cheap to clone (shared state).
#[derive(Clone)]
pub struct DeviceService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    settings: SettingsService,
    crypto: Arc<SecretBox>,
    strategies: StrategyService,
    presence: Presence,
}

impl DeviceService {
    pub fn new(
        db: DatabaseConnection,
        settings: SettingsService,
        crypto: Arc<SecretBox>,
        strategies: StrategyService,
        presence: Presence,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                settings,
                crypto,
                strategies,
                presence,
            }),
        }
    }

    pub fn presence(&self) -> &Presence {
        &self.inner.presence
    }

    /// The `devices.id` of `rustdesk_id` if `uuid` (base64) identifies it (see the module
    /// documentation). The identity of a known device is cached in [`Presence`], so this
    /// queries the database only for a device not seen since it was evicted, or an unknown id.
    pub async fn verify(&self, rustdesk_id: &str, uuid: &str) -> Result<Option<i64>, DeviceError> {
        let presence = &self.inner.presence;
        let identity = match presence.identity(rustdesk_id) {
            Some(identity) => identity,
            None => {
                let Some(row) = self.inner.find(rustdesk_id).await? else {
                    return Ok(None);
                };
                let identity = Identity {
                    device_id: row.id,
                    uuid: row.uuid,
                };
                presence.remember(rustdesk_id, identity.clone());
                identity
            }
        };
        let verified = self.inner.uuid_matches(&identity.uuid, uuid);
        Ok(verified.then_some(identity.device_id))
    }

    /// Stores a device's sysinfo unless it is unchanged (same content hash: no write at all),
    /// applying its presets with the first one stored (see `presets`).
    pub async fn report_sysinfo(
        &self,
        report: SysinfoReport,
    ) -> Result<SysinfoOutcome, DeviceError> {
        let report = SysinfoReport {
            sysinfo: report.sysinfo.sanitized(),
            ..report
        };
        let hash = report.sysinfo.hash();
        let Some(row) = self.inner.find(&report.rustdesk_id).await? else {
            return Ok(SysinfoOutcome::NotFound);
        };
        if !self.inner.uuid_matches(&row.uuid, &report.uuid) {
            return Ok(SysinfoOutcome::NotFound);
        }
        if row.sysinfo_hash.as_deref() == Some(hash.as_str()) {
            return Ok(SysinfoOutcome::Unchanged);
        }
        self.detached(move |inner| inner.store_sysinfo(report, hash))
            .await
    }

    /// Deletes device `device_id` (`devices.id`) and its strategy assignment; `false` if it
    /// did not exist. The signal server keeps its in-memory registration of the id until it
    /// restarts.
    pub async fn delete(&self, device_id: i64) -> Result<bool, DeviceError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let found = Devices::find_by_id(device_id)
                .lock_exclusive()
                .one(&tx)
                .await?;
            let Some(row) = found else {
                return Ok(false);
            };
            strategies::forget_device(&tx, row.id).await?;
            Devices::delete_by_id(row.id).exec(&tx).await?;
            tx.commit().await?;
            inner.strategies.cache().invalidate();
            inner.presence.forget(&row.rustdesk_id);
            Ok(true)
        })
        .await
    }

    /// Creates a device group; returns its id. Names are unique (exact match).
    pub async fn create_group(&self, name: &str, note: &str) -> Result<i64, DeviceError> {
        let name = validate_group_name(name)?;
        if let Some(reason) = text::problem(note, NOTE_MAX_LEN, true) {
            return Err(DeviceError::Invalid {
                field: "note",
                reason,
            });
        }
        let now = time::now();
        let row = device_group::ActiveModel {
            name: Set(name),
            note: Set(note.to_owned()),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let created = row.insert(&self.inner.db).await.map_err(name_conflict)?;
        Ok(created.id)
    }

    /// Deletes device group `id` and its strategy assignment; its devices become ungrouped
    /// (foreign key `SET NULL`). `false` if it did not exist.
    pub async fn delete_group(&self, id: i64) -> Result<bool, DeviceError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            strategies::forget_device_group(&tx, id).await?;
            let deleted = device_group::Entity::delete_by_id(id).exec(&tx).await?;
            tx.commit().await?;
            let existed = deleted.rows_affected > 0;
            if existed {
                inner.strategies.cache().invalidate();
            }
            Ok(existed)
        })
        .await
    }

    /// Runs `op` to completion in its own task (see the module documentation).
    async fn detached<T, F, Fut>(&self, op: F) -> Result<T, DeviceError>
    where
        F: FnOnce(Arc<Inner>) -> Fut,
        Fut: Future<Output = Result<T, DeviceError>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::spawn(op(Arc::clone(&self.inner)))
            .await
            .map_err(|_| DeviceError::Aborted)?
    }
}

impl Inner {
    async fn find(&self, rustdesk_id: &str) -> Result<Option<device::Model>, DbErr> {
        Devices::find()
            .filter(device::Column::RustdeskId.eq(rustdesk_id))
            .one(&self.db)
            .await
    }

    /// `given` (base64) is the stored uuid, or uuid verification is switched off.
    fn uuid_matches(&self, stored: &[u8], given: &str) -> bool {
        if !self.settings.snapshot().get(&DEVICES_VERIFY_UUID) {
            return true;
        }
        let Ok(given) = KEY_B64.decode(given) else {
            return false;
        };
        stored.ct_eq(&given).into()
    }

    /// The write half of [`DeviceService::report_sysinfo`], re-checked under the row lock.
    async fn store_sysinfo(
        self: Arc<Self>,
        report: SysinfoReport,
        hash: String,
    ) -> Result<SysinfoOutcome, DeviceError> {
        let scope = presets::scope(&self.settings.snapshot());
        let tx = begin_write(&self.db).await?;
        let found = Devices::find()
            .filter(device::Column::RustdeskId.eq(&report.rustdesk_id))
            .lock_exclusive()
            .one(&tx)
            .await?;
        // The row may have been deleted (and re-registered) since the unlocked read.
        let Some(row) = found.filter(|row| self.uuid_matches(&row.uuid, &report.uuid)) else {
            return Ok(SysinfoOutcome::NotFound);
        };
        if row.sysinfo_hash.as_deref() == Some(hash.as_str()) {
            return Ok(SysinfoOutcome::Unchanged);
        }
        // Decided under the row lock: of concurrent first uploads, one applies its presets.
        let first = row.sysinfo_hash.is_none() && scope != AcceptPresets::Off;
        let placed = match first {
            true => presets::resolve_row(&tx, &report.presets, &row).await?,
            false => RowPresets::default(),
        };
        let stored = write_row(&tx, row, &report.sysinfo, hash, &placed).await?;
        tx.commit().await?;
        if placed.moves_device() {
            self.strategies.cache().invalidate();
        }
        if first {
            self.apply_late_presets(&stored, &report.presets, scope)
                .await;
        }
        Ok(SysinfoOutcome::Stored)
    }

    /// The presets applied after the sysinfo commit: the strategy, and with
    /// [`AcceptPresets::All`] the address book entry.
    async fn apply_late_presets(&self, device: &device::Model, p: &Presets, scope: AcceptPresets) {
        if let Some(name) = &p.strategy_name {
            presets::apply_strategy(&self.strategies, device, name).await;
        }
        let book = p
            .address_book
            .as_ref()
            .filter(|_| scope == AcceptPresets::All);
        if let Some(preset) = book {
            let books = AddressBookService::new(
                self.db.clone(),
                self.settings.clone(),
                Arc::clone(&self.crypto),
            );
            preset_book::apply(&books, device, preset).await;
        }
    }
}

/// Writes the sysinfo columns, the hash and the row presets of `row`.
async fn write_row(
    tx: &DatabaseTransaction,
    row: device::Model,
    info: &Sysinfo,
    hash: String,
    placed: &RowPresets,
) -> Result<device::Model, DbErr> {
    let mut active = row.into_active_model();
    active.hostname = Set(info.hostname.clone());
    active.username = Set(info.username.clone());
    active.os = Set(info.os.clone());
    active.cpu = Set(info.cpu.clone());
    active.memory = Set(info.memory.clone());
    active.version = Set(info.version.clone());
    active.sysinfo_hash = Set(Some(hash));
    active.updated_at = Set(time::now());
    if let Some(owner) = placed.owner_user_id {
        active.owner_user_id = Set(Some(owner));
    }
    if let Some(group) = placed.device_group_id {
        active.device_group_id = Set(Some(group));
    }
    if let Some(note) = &placed.note {
        active.note = Set(Some(note.clone()));
    }
    active.update(tx).await
}

fn validate_group_name(raw: &str) -> Result<String, DeviceError> {
    let name = raw.trim();
    let problem = match name.is_empty() {
        true => Some("must not be blank".to_owned()),
        false => text::problem(name, DEVICE_GROUP_NAME_MAX_LEN, false),
    };
    match problem {
        Some(reason) => Err(DeviceError::Invalid {
            field: "name",
            reason,
        }),
        None => Ok(name.to_owned()),
    }
}

fn name_conflict(err: DbErr) -> DeviceError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => DeviceError::Conflict { field: "name" },
        _ => DeviceError::Database(err),
    }
}
