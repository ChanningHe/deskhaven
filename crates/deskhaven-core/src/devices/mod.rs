//! Device registry (04-data-model §4): `devices` rows keyed by RustDesk id.
//!
//! Rows are created by the signal server's first accepted RegisterPk of an id (07-hbbs-fork
//! §3.1) and bind that id to the device uuid for good: the uuid never changes afterwards
//! (06-security §2), only the public key and the registration address / time do.
//! `uuid` / `pk` are stored as raw bytes (`BLOB` / `BYTEA`), compared byte for byte.
//!
//! [`Presence`] tracks which devices are online (in memory only). [`DeviceService`] serves
//! the device endpoints: id + uuid verification, sysinfo with its deployment presets, and the
//! deletion of devices and device groups.

mod limits;
mod presence;
mod preset_book;
mod presets;
mod service;
mod sysinfo;

pub use limits::{SYSINFO_PER_IP_PER_MINUTE, SYSINFO_WRITE_INTERVAL};
pub use presence::{
    DEFAULT_CAPACITY, DevicePresence, HEARTBEAT_TTL, MAX_CONNS, Presence, SIGNAL_TTL,
    SWEEP_INTERVAL, Seen,
};
pub use preset_book::PRESET_TAG_COLOR;
pub use presets::{AddressBookPreset, Presets};
pub use service::{DEVICE_GROUP_NAME_MAX_LEN, DeviceService, SysinfoOutcome, SysinfoReport};
pub use sysinfo::{MAX_FIELD_CHARS, Sysinfo};

use std::net::IpAddr;

use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, TryInsertResult,
};

/// An accepted RegisterPk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub rustdesk_id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    /// Source address of the RegisterPk.
    pub ip: IpAddr,
    /// When it was accepted.
    pub at: TimestampMs,
}

/// What the signal server needs of a registered device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredKeys {
    pub rustdesk_id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    /// Source address of the last RegisterPk, as stored.
    pub register_ip: String,
}

/// Why a device operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceError {
    /// The id is registered with another uuid; nothing changed.
    #[error("device {rustdesk_id} is registered with another uuid")]
    UuidMismatch { rustdesk_id: String },

    /// Malformed input; `field` names the offending input field.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    /// The operation's task panicked or the runtime is shutting down; whether it committed
    /// is unknown until the next read.
    #[error("device operation did not complete")]
    Aborted,

    #[error("device storage failed")]
    Database(#[from] DbErr),
}

/// `devices` access. Cheap to clone (pooled connection).
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

    /// Up to `limit` registered devices with `rustdesk_id` after `after` (from the first if
    /// `None`), ordered by `rustdesk_id` (keyset pagination over `ux_devices_rustdesk_id`), for
    /// the signal server's startup preload. Order and comparison use the same collation, so
    /// passing the last id of a page as `after` walks every row exactly once.
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

    /// Inserts the device on its first registration; afterwards updates `pk`, `register_ip`
    /// and `register_at`, but only while the stored uuid matches (otherwise
    /// [`DeviceError::UuidMismatch`], nothing changed).
    ///
    /// Each statement is atomic on both backends, so concurrent calls need no lock: the
    /// update is conditional on the uuid, and the insert does nothing when it conflicts on
    /// the unique `rustdesk_id`. The common case (re-registration) is a single UPDATE, which
    /// on PostgreSQL also avoids burning a sequence value on a conflicting INSERT.
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

    /// `true` if a row with this id and uuid was updated.
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

    /// `true` if `reg` created the row, `false` if the id already existed.
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
