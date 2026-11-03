//! Wire types of the device group endpoints.

use deskhaven_core::devices::DeviceGroupSummary;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::groups::ManagedByView;

/// A device group with its number of devices. Timestamps are unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeviceGroupView {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedByView,
    pub device_count: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<DeviceGroupSummary> for DeviceGroupView {
    fn from(s: DeviceGroupSummary) -> Self {
        Self {
            id: s.group.id,
            name: s.group.name,
            note: s.group.note,
            managed_by: s.group.managed_by.into(),
            device_count: s.device_count,
            created_at: s.group.created_at.timestamp_millis(),
            updated_at: s.group.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewDeviceGroupBody {
    /// 1-64 characters, unique.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub note: String,
}

/// Changes to a device group; absent fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DeviceGroupPatch {
    #[garde(length(chars, min = 1, max = 64))]
    pub name: Option<String>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
}
