//! `device_strategy_keys`: the strategy option keys the heartbeat last sent to a device with a
//! non-empty value (M2-14), so that a key which later disappears from the device's effective
//! strategy can be sent once more as `""` (the client only touches the keys it receives).
//!
//! A superset of what the device may hold: grown before a strategy is sent, shrunk once the
//! device confirms it. Dropped with the device (`ON DELETE CASCADE`).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_strategy_keys")]
pub struct Model {
    /// `devices.id`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_id: i64,
    /// JSON array of option names, sorted and unique (compared as text when updated).
    #[sea_orm(column_type = "Text")]
    pub option_keys: String,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::device::Entity",
        from = "Column::DeviceId",
        to = "super::device::Column::Id",
        on_delete = "Cascade"
    )]
    Device,
}

impl Related<super::device::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
