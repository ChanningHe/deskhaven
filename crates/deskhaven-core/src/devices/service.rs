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
use std::sync::{Arc, PoisonError, RwLock};

use deskhaven_config::settings::{AcceptPresets, DEVICES_VERIFY_UUID};
use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, Entity as Devices};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect,
};
use subtle::ConstantTimeEq;

mod admin;
mod assign;
mod deploy;
mod heartbeat;

pub use admin::{
    ConnectionRecord, DeviceGroup, DeviceGroupQuery, DeviceGroupSort, DeviceGroupSortField,
    DeviceGroupSummary, DeviceGroupUpdate, DeviceQuery, DeviceRecord, DeviceSort, DeviceSortField,
    DeviceSummary, DeviceUpdate,
};
pub use assign::DeviceAssignment;
pub use deploy::{DEPLOY_ID_MAX_LEN, DEPLOY_ID_MIN_LEN, DeployOutcome, DeployRequest};
pub use heartbeat::{Heartbeat, HeartbeatOutcome, HeartbeatReply, StrategyPush};

use super::DeviceError;
use super::disconnects::DisconnectQueue;
use super::limits::{HEARTBEAT_PER_IP_PER_MINUTE, SYSINFO_PER_IP_PER_MINUTE, WriteLimit};
use super::presence::{Identity, Presence};
use super::preset_book;
use super::presets::{self, Presets, RowPresets};
use super::strategy_keys::StrategyDeliveries;
use super::sysinfo::Sysinfo;
use crate::addressbooks::AddressBookService;
use crate::auth::limiter::{ClientAddr, IpLimiter};
use crate::clock::SharedClock;
use crate::console_audit::TxGuard;
use crate::crypto::{SecretBox, b64::KEY_B64};
use crate::settings::SettingsService;
use crate::strategies::StrategyService;
use base64_022::Engine as _;
use std::time::Duration;

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
    /// Changed content, but the device wrote within
    /// [`SYSINFO_WRITE_INTERVAL`](super::SYSINFO_WRITE_INTERVAL): nothing
    /// written (see [`limits`](super::limits)).
    TooFrequent,
}

/// Called with the RustDesk id of each deleted device, after the commit (see
/// [`DeviceService::on_device_deleted`]). Must be cheap and must not panic.
pub type DeletedHook = Arc<dyn Fn(&str) + Send + Sync>;

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
    disconnects: DisconnectQueue,
    deliveries: StrategyDeliveries,
    sysinfo_ips: IpLimiter,
    heartbeat_ips: IpLimiter,
    sysinfo_writes: WriteLimit,
    deleted_hooks: RwLock<Vec<DeletedHook>>,
}

impl DeviceService {
    /// `presence` and the rate limits run on `clock` (share it with the other services).
    pub fn new(
        db: DatabaseConnection,
        settings: SettingsService,
        crypto: Arc<SecretBox>,
        strategies: StrategyService,
        clock: SharedClock,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                deliveries: StrategyDeliveries::new(db.clone()),
                db,
                settings,
                crypto,
                strategies,
                presence: Presence::new(Arc::clone(&clock)),
                disconnects: DisconnectQueue::new(Arc::clone(&clock)),
                sysinfo_ips: IpLimiter::fixed(SYSINFO_PER_IP_PER_MINUTE, Arc::clone(&clock)),
                heartbeat_ips: IpLimiter::fixed(HEARTBEAT_PER_IP_PER_MINUTE, Arc::clone(&clock)),
                sysinfo_writes: WriteLimit::new(clock),
                deleted_hooks: RwLock::default(),
            }),
        }
    }

    pub fn presence(&self) -> &Presence {
        &self.inner.presence
    }

    /// Disconnect requests, handed to the devices by their next heartbeat.
    pub fn disconnects(&self) -> &DisconnectQueue {
        &self.inner.disconnects
    }

    /// Runs `hook` after every committed device deletion. `serve` uses it to make the signal
    /// server forget the id (`SignalHandle::reload_peer`), so the device re-registers as new
    /// at once instead of being answered `ID_NOT_FOUND` until a restart.
    pub fn on_device_deleted(&self, hook: DeletedHook) {
        let mut hooks = self
            .inner
            .deleted_hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        hooks.push(hook);
    }

    /// Takes a token of `client`'s sysinfo bucket, or returns how long until one is free
    /// ([`SYSINFO_PER_IP_PER_MINUTE`]).
    pub fn admit_sysinfo(&self, client: ClientAddr) -> Result<(), Duration> {
        self.inner.sysinfo_ips.check(client)
    }

    /// Takes a token of `client`'s heartbeat bucket, or returns how long until one is free
    /// ([`HEARTBEAT_PER_IP_PER_MINUTE`](super::HEARTBEAT_PER_IP_PER_MINUTE)).
    pub fn admit_heartbeat(&self, client: ClientAddr) -> Result<(), Duration> {
        self.inner.heartbeat_ips.check(client)
    }

    /// The `devices.id` of `rustdesk_id` if `uuid` (base64) identifies it (see the module
    /// documentation). The identity of a known device is cached in [`Presence`], so this
    /// queries the database only for a device not seen since it was evicted, or an unknown id.
    pub async fn verify(&self, rustdesk_id: &str, uuid: &str) -> Result<Option<i64>, DeviceError> {
        let identity = self.identify(rustdesk_id, uuid).await?;
        Ok(identity.map(|identity| identity.device_id))
    }

    /// Whether `rustdesk_id` is a registered device, without a uuid (the controller's audit
    /// note carries none). Served from the same identity cache as [`verify`](Self::verify).
    pub async fn is_registered(&self, rustdesk_id: &str) -> Result<bool, DeviceError> {
        if self.inner.presence.identity(rustdesk_id).is_some() {
            return Ok(true);
        }
        Ok(self.load_identity(rustdesk_id).await?.is_some())
    }

    /// [`verify`](Self::verify) with the whole cached identity.
    async fn identify(
        &self,
        rustdesk_id: &str,
        uuid: &str,
    ) -> Result<Option<Identity>, DeviceError> {
        let identity = match self.inner.presence.identity(rustdesk_id) {
            Some(identity) => Some(identity),
            None => self.load_identity(rustdesk_id).await?,
        };
        Ok(identity.filter(|identity| self.inner.uuid_matches(&identity.uuid, uuid)))
    }

    /// The identity of `rustdesk_id` read from the database (`None`: unknown id) and cached.
    async fn load_identity(&self, rustdesk_id: &str) -> Result<Option<Identity>, DeviceError> {
        let presence = &self.inner.presence;
        let epoch = presence.identity_epoch();
        let Some(row) = self.inner.find(rustdesk_id).await? else {
            return Ok(None);
        };
        let identity = Identity {
            device_id: row.id,
            uuid: row.uuid,
            has_sysinfo: row.sysinfo_hash.is_some(),
        };
        presence.remember(rustdesk_id, identity.clone(), epoch);
        Ok(Some(identity))
    }

    /// Stores a device's sysinfo unless it is unchanged (same content hash: no write at all),
    /// applying its presets with the first one stored (see `presets`).
    pub async fn report_sysinfo(
        &self,
        report: SysinfoReport,
    ) -> Result<SysinfoOutcome, DeviceError> {
        let rustdesk_id = report.rustdesk_id.clone();
        let outcome = self.store_report(report).await?;
        if matches!(outcome, SysinfoOutcome::Stored | SysinfoOutcome::Unchanged) {
            // The heartbeat stops asking for it (see `heartbeat`).
            self.inner.presence.mark_sysinfo(&rustdesk_id);
        }
        Ok(outcome)
    }

    async fn store_report(&self, report: SysinfoReport) -> Result<SysinfoOutcome, DeviceError> {
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
        if !self.inner.sysinfo_writes.try_acquire(&report.rustdesk_id) {
            return Ok(SysinfoOutcome::TooFrequent);
        }
        self.detached(move |inner| inner.store_sysinfo(report, hash))
            .await
    }

    /// Deletes device `device_id` (`devices.id`) and its strategy assignment; `false` if it
    /// did not exist. Then runs the [`on_device_deleted`](Self::on_device_deleted) hooks.
    pub async fn delete(&self, device_id: i64) -> Result<bool, DeviceError> {
        self.delete_guarded(device_id, TxGuard::none()).await
    }

    /// Creates a device group; returns its id. Names are unique (exact match).
    pub async fn create_group(&self, name: &str, note: &str) -> Result<i64, DeviceError> {
        let group = self
            .create_group_guarded(name, note, TxGuard::none())
            .await?;
        Ok(group.id)
    }

    /// Deletes device group `id` and its strategy assignment; its devices become ungrouped
    /// (foreign key `SET NULL`). `false` if it did not exist; a group defined in the
    /// configuration file is not deleted ([`DeviceError::ManagedByConfig`]).
    pub async fn delete_group(&self, id: i64) -> Result<bool, DeviceError> {
        match self.delete_group_guarded(id, TxGuard::none()).await {
            Ok(()) => Ok(true),
            Err(DeviceError::GroupNotFound) => Ok(false),
            Err(other) => Err(other),
        }
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
    fn run_deleted_hooks(&self, rustdesk_id: &str) {
        let hooks = self
            .deleted_hooks
            .read()
            .unwrap_or_else(PoisonError::into_inner);
        for hook in hooks.iter() {
            hook(rustdesk_id);
        }
    }

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
