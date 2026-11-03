//! `devices`: the device registry (04-data-model §4), keyed by the RustDesk id.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

/// `devices.status` (TEXT + CHECK).
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum DeviceStatus {
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "disabled")]
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "devices")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// hbbs `id`; unique.
    #[sea_orm(column_type = "Text", unique)]
    pub rustdesk_id: String,
    /// Device uuid from its first RegisterPk; immutable.
    #[sea_orm(column_type = "Blob")]
    pub uuid: Vec<u8>,
    /// Device public key from its last RegisterPk.
    #[sea_orm(column_type = "Blob")]
    pub pk: Vec<u8>,
    #[sea_orm(column_type = "Text", nullable)]
    pub hostname: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub username: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub os: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub cpu: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub memory: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub version: Option<String>,
    /// Content hash of the last stored sysinfo (unchanged sysinfo is not written).
    #[sea_orm(column_type = "Text", nullable)]
    pub sysinfo_hash: Option<String>,
    /// `None` = unassigned; cleared when the user is deleted.
    pub owner_user_id: Option<i64>,
    /// `None` = ungrouped; cleared when the group is deleted.
    pub device_group_id: Option<i64>,
    #[sea_orm(column_type = "Text", nullable)]
    pub note: Option<String>,
    /// The administrator's name for the device; sysinfo never writes it.
    #[sea_orm(column_type = "Text", nullable)]
    pub display_name: Option<String>,
    /// The administrator's name for the device's user; sysinfo never writes it.
    #[sea_orm(column_type = "Text", nullable)]
    pub display_username: Option<String>,
    pub status: DeviceStatus,
    /// `None` = not deployed (P1 admission).
    pub deployed_at: Option<TimestampMs>,
    pub first_seen_at: TimestampMs,
    /// Flushed in batches from the in-memory presence table.
    pub last_seen_at: TimestampMs,
    #[sea_orm(column_type = "Text")]
    pub last_ip: String,
    /// Source IP of the last RegisterPk.
    #[sea_orm(column_type = "Text")]
    pub register_ip: String,
    /// Time of the last RegisterPk.
    pub register_at: TimestampMs,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::OwnerUserId",
        to = "super::user::Column::Id",
        on_delete = "SetNull"
    )]
    Owner,
    #[sea_orm(
        belongs_to = "super::device_group::Entity",
        from = "Column::DeviceGroupId",
        to = "super::device_group::Column::Id",
        on_delete = "SetNull"
    )]
    DeviceGroup,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Owner.def()
    }
}

impl Related<super::device_group::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DeviceGroup.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
