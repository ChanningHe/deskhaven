//! `settings_history`: append-only log of setting changes.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "settings_history")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(column_type = "Text")]
    pub key: String,
    /// JSON-encoded previous value; `None` when the key had no stored value.
    #[sea_orm(column_type = "Text", nullable)]
    pub old_value: Option<String>,
    /// JSON-encoded new value.
    #[sea_orm(column_type = "Text")]
    pub new_value: String,
    pub version: i64,
    pub updated_by: Option<i64>,
    pub updated_at: TimestampMs,
    #[sea_orm(column_type = "Text", nullable)]
    pub reason: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
