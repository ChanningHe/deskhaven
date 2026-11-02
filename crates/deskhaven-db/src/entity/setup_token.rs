//! `setup_tokens`: one-time first-start Owner setup link (hash only).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "setup_tokens")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub token_hash: String,
    pub created_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub used_at: Option<TimestampMs>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
