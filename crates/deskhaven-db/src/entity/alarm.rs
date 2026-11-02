//! `alarms`: security alarms raised by a device (blocked IP, too many attempts, ...).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "alarms")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub device_rustdesk_id: String,
    /// Alarm type as sent by the client; unknown values are kept as they come.
    pub typ: i32,
    /// JSON object.
    pub info: String,
    pub conn_id: i64,
    pub created_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
