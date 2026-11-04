//! Fleet totals for the overview page (M3-14): device counts, live connections and the
//! distribution of the client versions the devices report.
//!
//! Online devices and live connections come from the presence table (no query); the totals
//! and the distribution are two aggregate queries and one count over `devices`.

use deskhaven_db::entity::device;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

use crate::devices::DeviceError;
use crate::devices::service::DeviceService;

/// The devices that report one client version, for the distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionCount {
    /// Self-reported by the device (06 §2): never trust it.
    pub version: String,
    pub count: u64,
}

/// What [`DeviceService::fleet`] reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FleetStats {
    pub total: u64,
    /// Devices online now.
    pub online: u64,
    /// Connections those devices reported in their latest heartbeat.
    pub connections: u64,
    /// The most common client versions, most devices first, ties in version order.
    pub top_versions: Vec<VersionCount>,
    /// Devices whose version is not among `top_versions`.
    pub other_versions: u64,
    /// Devices that never reported a version.
    pub unknown_versions: u64,
}

impl DeviceService {
    /// Fleet totals; the version distribution lists the `top` most common versions.
    pub async fn fleet(&self, top: u64) -> Result<FleetStats, DeviceError> {
        let db = &self.inner.db;
        let reported = device::Column::Version
            .is_not_null()
            .and(device::Column::Version.ne(""));
        let total = device::Entity::find().count(db).await?;
        let known = device::Entity::find()
            .filter(reported.clone())
            .count(db)
            .await?;
        let counts: Vec<(String, i64)> = device::Entity::find()
            .select_only()
            .column(device::Column::Version)
            .column_as(Expr::col(device::Column::Id).count(), "devices")
            .filter(reported)
            .group_by(device::Column::Version)
            .order_by(Expr::col("devices"), Order::Desc)
            .order_by(device::Column::Version, Order::Asc)
            .limit(top)
            .into_tuple()
            .all(db)
            .await?;
        let top_versions: Vec<VersionCount> = counts
            .into_iter()
            .map(|(version, count)| VersionCount {
                version,
                count: u64::try_from(count).unwrap_or(0),
            })
            .collect();
        let listed: u64 = top_versions.iter().map(|v| v.count).sum();
        let live = self.inner.presence.live();
        Ok(FleetStats {
            total,
            online: live.devices as u64,
            connections: live.connections as u64,
            other_versions: known.saturating_sub(listed),
            unknown_versions: total.saturating_sub(known),
            top_versions,
        })
    }
}
