//! `file_transfers`: audit record of a file transfer reported by the controlled device.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

/// Direction as seen from the reporting (controlled) device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Direction {
    #[sea_orm(string_value = "send")]
    Send,
    #[sea_orm(string_value = "receive")]
    Receive,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "file_transfers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub device_rustdesk_id: String,
    pub peer_rustdesk_id: String,
    pub conn_id: i64,
    pub direction: Direction,
    pub path: String,
    pub is_file: bool,
    pub file_count: i32,
    /// JSON array of `[name, size]` pairs (the client sends at most 10).
    pub files: String,
    pub ip: Option<String>,
    pub created_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
