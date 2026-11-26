//! A row binds its RustDesk id to the device uuid for good: only the public key and the
//! registration address change. `uuid` and `pk` are raw bytes, compared byte for byte.

mod disconnects;
mod flush;
mod limits;
mod new_ids;
mod presence;
mod preset_book;
mod presets;
mod service;
mod strategy_keys;
mod sysinfo;

pub use disconnects::{DISCONNECT_TTL, DisconnectQueue, MAX_QUEUED_DEVICES, QueueFull};
pub use flush::{FLUSH_BATCH, FlushReport, LastSeenFlusher, MAX_RETRY};
pub use limits::{
    HEARTBEAT_MIN_INTERVAL, HEARTBEAT_PER_IP_PER_MINUTE, SYSINFO_PER_IP_PER_MINUTE,
    SYSINFO_WRITE_INTERVAL,
};
pub use new_ids::{DEPLOY_NEW_IDS_PER_ACTOR_PER_DAY, DEPLOY_NEW_IDS_PER_IP_PER_DAY};
pub use presence::{
    DEFAULT_CAPACITY, DevicePresence, HEARTBEAT_TTL, LiveCounts, MAX_CONNS, Presence, SIGNAL_TTL,
    SWEEP_INTERVAL, Seen,
};
pub use preset_book::PRESET_TAG_COLOR;
pub use presets::{AddressBookPreset, Presets};
pub use service::{
    ConnectionRecord, DEPLOY_ID_MAX_LEN, DEPLOY_ID_MIN_LEN, DEVICE_GROUP_NAME_MAX_LEN, DeletedHook,
    DeployOutcome, DeployRequest, DeviceAssignment, DeviceGroup, DeviceGroupQuery, DeviceGroupSort,
    DeviceGroupSortField, DeviceGroupSummary, DeviceGroupUpdate, DeviceQuery, DeviceRecord,
    DeviceService, DeviceSort, DeviceSortField, DeviceSummary, DeviceUpdate, FleetStats, Heartbeat,
    HeartbeatOutcome, HeartbeatReply, IdHolder, LIVE_RETRY_AFTER, LiveLookup, LiveRegistrations,
    LiveUnknown, Requester, StrategyPush, SysinfoOutcome, SysinfoReport, VersionCount,
};
pub use strategy_keys::MAX_PENDING;
pub use sysinfo::{MAX_FIELD_CHARS, Sysinfo};

use std::net::IpAddr;

use crate::console_audit::{GuardError, Refused};
use deskhaven_db::entity::device;
pub use deskhaven_db::entity::device::DeviceStatus;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, TryInsertResult,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub rustdesk_id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub ip: IpAddr,
    pub at: TimestampMs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredKeys {
    pub rustdesk_id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub register_ip: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("device {rustdesk_id} is registered with another uuid")]
    UuidMismatch { rustdesk_id: String },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    #[error("device not found")]
    NotFound,

    #[error("device group not found")]
    GroupNotFound,

    #[error("the device group is managed by the configuration file")]
    ManagedByConfig,

    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("too many devices have pending disconnects")]
    QueueFull,
    #[error("unknown device id or uuid")]
    UnknownDevice,

    #[error("{field} does not name an existing {kind}")]
    NameNotFound {
        field: &'static str,
        kind: &'static str,
    },

    #[error("device strategy assignment failed")]
    Strategy(#[source] crate::strategies::StrategyError),

    #[error("device address book entry failed")]
    AddressBook(#[source] crate::addressbooks::AddressBookError),

    /// Whether it committed is unknown until the next read.
    #[error("device operation did not complete")]
    Aborted,

    #[error("device storage failed")]
    Database(#[from] DbErr),
}

impl From<GuardError> for DeviceError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceRepository {
    db: DatabaseConnection,
}

impl DeviceRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub async fn find_by_rustdesk_id(
        &self,
        rustdesk_id: &str,
    ) -> Result<Option<device::Model>, DbErr> {
        device::Entity::find()
            .filter(device::Column::RustdeskId.eq(rustdesk_id))
            .one(&self.db)
            .await
    }

    /// Order and comparison use the same collation, so passing a page's last id as `after` walks
    /// every row exactly once.
    pub async fn registrations_page(
        &self,
        after: Option<&str>,
        limit: u64,
    ) -> Result<Vec<RegisteredKeys>, DbErr> {
        let mut query = device::Entity::find()
            .select_only()
            .columns([
                device::Column::RustdeskId,
                device::Column::Uuid,
                device::Column::Pk,
                device::Column::RegisterIp,
            ])
            .order_by_asc(device::Column::RustdeskId)
            .limit(limit);
        if let Some(after) = after {
            query = query.filter(device::Column::RustdeskId.gt(after));
        }
        let rows: Vec<(String, Vec<u8>, Vec<u8>, String)> =
            query.into_tuple().all(&self.db).await?;
        Ok(rows
            .into_iter()
            .map(|(rustdesk_id, uuid, pk, register_ip)| RegisteredKeys {
                rustdesk_id,
                uuid,
                pk,
                register_ip,
            })
            .collect())
    }

    /// Each statement is atomic on both backends, so concurrent calls need no lock. Re-registration
    /// is a single UPDATE, which on PostgreSQL also avoids burning a sequence value.
    pub async fn upsert_registration(&self, reg: &Registration) -> Result<(), DeviceError> {
        if self.update_keys(reg).await? || self.insert_new(reg).await? {
            return Ok(());
        }
        // Lost an insert race: fine if the winner registered the same uuid.
        if self.update_keys(reg).await? {
            return Ok(());
        }
        Err(DeviceError::UuidMismatch {
            rustdesk_id: reg.rustdesk_id.clone(),
        })
    }

    async fn update_keys(&self, reg: &Registration) -> Result<bool, DbErr> {
        let updated = device::Entity::update_many()
            .col_expr(device::Column::Pk, Expr::value(reg.pk.clone()))
            .col_expr(device::Column::RegisterIp, Expr::value(reg.ip.to_string()))
            .col_expr(device::Column::RegisterAt, Expr::value(reg.at))
            .col_expr(device::Column::UpdatedAt, Expr::value(time::now()))
            .filter(device::Column::RustdeskId.eq(&reg.rustdesk_id))
            .filter(device::Column::Uuid.eq(reg.uuid.clone()))
            .exec(&self.db)
            .await?;
        Ok(updated.rows_affected > 0)
    }

    async fn insert_new(&self, reg: &Registration) -> Result<bool, DbErr> {
        let ip = reg.ip.to_string();
        let row = device::ActiveModel {
            id: NotSet,
            rustdesk_id: Set(reg.rustdesk_id.clone()),
            uuid: Set(reg.uuid.clone()),
            pk: Set(reg.pk.clone()),
            status: Set(DeviceStatus::Active),
            first_seen_at: Set(reg.at),
            last_seen_at: Set(reg.at),
            last_ip: Set(ip.clone()),
            register_ip: Set(ip),
            register_at: Set(reg.at),
            created_at: Set(time::now()),
            updated_at: Set(time::now()),
            ..Default::default()
        };
        let res = device::Entity::insert(row)
            .on_conflict_do_nothing_on([device::Column::RustdeskId])
            .exec_without_returning(&self.db)
            .await?;
        Ok(matches!(res, TryInsertResult::Inserted(n) if n > 0))
    }
}
