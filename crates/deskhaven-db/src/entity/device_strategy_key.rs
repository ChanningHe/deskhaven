//! Option keys last sent to a device with a non-empty value, so a key that later leaves its
//! effective strategy can be sent once more as `""` (the client only touches keys it receives).
//! No foreign key: deleting a device does not reset its client, which re-registers as a new
//! `devices` row and must still get the keys it holds deleted.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "device_strategy_keys")]
pub struct Model {
    /// `devices.rustdesk_id` (the row may be gone or replaced).
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub rustdesk_id: String,
    /// JSON array of option names, sorted and unique (compared as text when updated).
    #[sea_orm(column_type = "Text")]
    pub option_keys: String,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
