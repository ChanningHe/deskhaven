//! Wire types of the device endpoints. Timestamps are unix milliseconds (UTC). Sysinfo
//! fields are what the device reported about itself (06 §2): the console labels them so.

use deskhaven_core::devices::{ConnectionRecord, DeviceStatus, DeviceSummary, DeviceUpdate};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::extract::present;

/// `devices.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatusView {
    Active,
    Disabled,
}

impl From<DeviceStatus> for DeviceStatusView {
    fn from(status: DeviceStatus) -> Self {
        match status {
            DeviceStatus::Active => Self::Active,
            DeviceStatus::Disabled => Self::Disabled,
        }
    }
}

impl From<DeviceStatusView> for DeviceStatus {
    fn from(status: DeviceStatusView) -> Self {
        match status {
            DeviceStatusView::Active => Self::Active,
            DeviceStatusView::Disabled => Self::Disabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OwnerRef {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeviceGroupRef {
    pub id: i64,
    pub name: String,
}

/// A device as administrators see it. Carries neither the uuid nor the public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeviceView {
    pub id: i64,
    /// The id clients connect to.
    pub rustdesk_id: String,
    /// Device-reported.
    pub hostname: Option<String>,
    /// Device-reported: the OS user running the client.
    pub username: Option<String>,
    /// Device-reported.
    pub os: Option<String>,
    /// Device-reported.
    pub cpu: Option<String>,
    /// Device-reported.
    pub memory: Option<String>,
    /// Device-reported: the client version.
    pub version: Option<String>,
    pub owner: Option<OwnerRef>,
    pub device_group: Option<DeviceGroupRef>,
    pub note: String,
    /// `disabled` is a marker only: it does not block the device yet.
    pub status: DeviceStatusView,
    /// Signal registration within 30 s or heartbeat within 45 s (from memory, not stored).
    pub online: bool,
    /// Latest contact: the presence table when it is newer than the stored value.
    pub last_seen_at: i64,
    pub last_ip: String,
    pub first_seen_at: i64,
    pub created_at: i64,
}

impl From<DeviceSummary> for DeviceView {
    fn from(s: DeviceSummary) -> Self {
        let record = s.record;
        let presence = s.presence;
        let stored_seen = record.last_seen_at.timestamp_millis();
        let live_seen = presence.as_ref().and_then(|p| p.last_seen_ms);
        let live_ip = presence.as_ref().and_then(|p| p.ip);
        Self {
            id: record.id,
            rustdesk_id: record.rustdesk_id,
            hostname: record.hostname,
            username: record.username,
            os: record.os,
            cpu: record.cpu,
            memory: record.memory,
            version: record.version,
            owner: owner_ref(record.owner_user_id, s.owner_username),
            device_group: group_ref(record.device_group_id, s.group_name),
            note: record.note.unwrap_or_default(),
            status: record.status.into(),
            online: presence.as_ref().is_some_and(|p| p.online),
            last_seen_at: live_seen.map_or(stored_seen, |live| live.max(stored_seen)),
            last_ip: live_ip.map_or(record.last_ip, |ip| ip.to_string()),
            first_seen_at: record.first_seen_at.timestamp_millis(),
            created_at: record.created_at.timestamp_millis(),
        }
    }
}

fn owner_ref(id: Option<i64>, username: Option<String>) -> Option<OwnerRef> {
    Some(OwnerRef {
        id: id?,
        username: username?,
    })
}

fn group_ref(id: Option<i64>, name: Option<String>) -> Option<DeviceGroupRef> {
    Some(DeviceGroupRef {
        id: id?,
        name: name?,
    })
}

/// One connection to a device, as the device reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConnectionView {
    pub guid: String,
    pub conn_id: i64,
    /// The controlling device and the name it reported.
    pub peer_rustdesk_id: Option<String>,
    pub peer_name: Option<String>,
    pub controller_user_id: Option<i64>,
    /// 0 remote control, 1 file transfer, 2 port forward, 3 camera, 4 terminal; other values
    /// come from newer clients.
    pub conn_type: Option<i32>,
    pub ip: Option<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl From<ConnectionRecord> for ConnectionView {
    fn from(c: ConnectionRecord) -> Self {
        Self {
            guid: c.guid,
            conn_id: c.conn_id,
            peer_rustdesk_id: c.peer_rustdesk_id,
            peer_name: c.peer_name,
            controller_user_id: c.controller_user_id,
            conn_type: c.conn_type,
            ip: c.ip,
            started_at: c.started_at.map(|t| t.timestamp_millis()),
            ended_at: c.ended_at.map(|t| t.timestamp_millis()),
        }
    }
}

/// Changes to a device; absent fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DevicePatch {
    /// The owner's user id; `null` removes the owner.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    #[garde(skip)]
    pub owner_user_id: Option<Option<i64>>,
    /// The device group's id; `null` ungroups the device.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i64>, nullable)]
    #[garde(skip)]
    pub device_group_id: Option<Option<i64>>,
    /// An empty string removes the note.
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
    /// `disabled` is a marker only: it does not block the device yet.
    #[garde(skip)]
    pub status: Option<DeviceStatusView>,
}

impl From<DevicePatch> for DeviceUpdate {
    fn from(p: DevicePatch) -> Self {
        Self {
            owner_user_id: p.owner_user_id,
            device_group_id: p.device_group_id,
            note: p.note,
            status: p.status.map(Into::into),
        }
    }
}

/// A connection to close, by the id the device reports in its heartbeat.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DisconnectBody {
    #[garde(range(min = 0))]
    #[schema(format = Int32)]
    pub conn_id: i32,
}

/// The request is queued; the device closes the connection at its next heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct DisconnectQueued {
    /// How long the device has to pick it up before the request expires.
    pub ttl_seconds: u64,
}
