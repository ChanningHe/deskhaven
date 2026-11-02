//! Device registry (04-data-model §4): `devices` rows keyed by RustDesk id.
//!
//! Rows are created by the signal server's first accepted RegisterPk of an id (07-hbbs-fork
//! §3.1) and bind that id to the device uuid for good: the uuid never changes afterwards
//! (06-security §2), only the public key and the registration address / time do.

use std::net::IpAddr;

use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::NotSet, ActiveValue::Set, ColumnTrait, DatabaseConnection, DbErr, EntityTrait,
    QueryFilter, TryInsertResult,
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

/// Why a registration was not stored.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeviceError {
    /// The id is registered with another uuid; nothing changed.
    #[error("device {rustdesk_id} is registered with another uuid")]
    UuidMismatch { rustdesk_id: String },

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

    /// Inserts the device on its first registration; afterwards updates `pk`, `register_ip`
    /// and `register_at`, but only while the stored uuid matches (otherwise
    /// [`DeviceError::UuidMismatch`]).
    ///
    /// Two statements, each atomic on both backends, so concurrent calls need no lock: an
    /// insert that conflicts on the unique `rustdesk_id` does nothing, and the update is
    /// conditional on the uuid.
    pub async fn upsert_registration(&self, reg: &Registration) -> Result<(), DeviceError> {
        if self.insert_new(reg).await? {
            return Ok(());
        }
        let updated = device::Entity::update_many()
            .col_expr(device::Column::Pk, Expr::value(reg.pk.clone()))
            .col_expr(device::Column::RegisterIp, Expr::value(reg.ip.to_string()))
            .col_expr(device::Column::RegisterAt, Expr::value(reg.at))
            .col_expr(device::Column::UpdatedAt, Expr::value(time::now()))
            .filter(device::Column::RustdeskId.eq(&reg.rustdesk_id))
            .filter(device::Column::Uuid.eq(reg.uuid.clone()))
            .exec(&self.db)
            .await?;
        if updated.rows_affected == 0 {
            return Err(DeviceError::UuidMismatch {
                rustdesk_id: reg.rustdesk_id.clone(),
            });
        }
        Ok(())
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
