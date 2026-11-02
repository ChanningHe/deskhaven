//! `strategies`: named sets of client options pushed through the heartbeat.

use sea_orm::entity::prelude::*;

use super::managed_by::ManagedBy;
use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "strategies")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// JSON object `{key: value}`; an empty-string value removes the option on the client.
    pub options: String,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::strategy_assignment::Entity")]
    Assignment,
}

impl Related<super::strategy_assignment::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Assignment.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
