//! What the web console needs of the registry (M3-05): paged device lists with presence,
//! device details, the console's writes (owner, device group, note, status, delete) and
//! device groups. Writes take a [`TxGuard`](crate::console_audit::TxGuard) that records the
//! console audit row in the write's own transaction.

mod connections;
mod fleet;
mod groups;
mod list;
mod write;

pub use connections::ConnectionRecord;
pub use fleet::{FleetStats, VersionCount};
pub use groups::{
    DeviceGroup, DeviceGroupQuery, DeviceGroupSort, DeviceGroupSortField, DeviceGroupSummary,
    DeviceGroupUpdate,
};
pub use list::{DeviceQuery, DeviceSort, DeviceSortField, DeviceSummary};
pub use write::DeviceUpdate;

use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::time::TimestampMs;

/// A `devices` row as the console sees it: everything but the identity secrets (`uuid`,
/// `pk`) and the sysinfo hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRecord {
    pub id: i64,
    pub rustdesk_id: String,
    /// Sysinfo, self-reported by the device (06 §2): never trust it.
    pub hostname: Option<String>,
    pub username: Option<String>,
    pub os: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub version: Option<String>,
    pub owner_user_id: Option<i64>,
    pub device_group_id: Option<i64>,
    pub note: Option<String>,
    pub status: DeviceStatus,
    pub first_seen_at: TimestampMs,
    /// As flushed from the presence table; the live value is in [`DeviceSummary::presence`].
    pub last_seen_at: TimestampMs,
    pub last_ip: String,
    pub register_ip: String,
    pub register_at: TimestampMs,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<device::Model> for DeviceRecord {
    fn from(m: device::Model) -> Self {
        Self {
            id: m.id,
            rustdesk_id: m.rustdesk_id,
            hostname: m.hostname,
            username: m.username,
            os: m.os,
            cpu: m.cpu,
            memory: m.memory,
            version: m.version,
            owner_user_id: m.owner_user_id,
            device_group_id: m.device_group_id,
            note: m.note,
            status: m.status,
            first_seen_at: m.first_seen_at,
            last_seen_at: m.last_seen_at,
            last_ip: m.last_ip,
            register_ip: m.register_ip,
            register_at: m.register_at,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}
