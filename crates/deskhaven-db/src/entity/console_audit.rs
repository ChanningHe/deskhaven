//! `actor_id` names a user or an API token depending on `actor_kind`, so it has no foreign
//! key: the log must outlive its actors. Secret values are masked before they are written.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum ActorKind {
    #[sea_orm(string_value = "user")]
    User,
    #[sea_orm(string_value = "api_token")]
    ApiToken,
    #[sea_orm(string_value = "cli")]
    Cli,
    #[sea_orm(string_value = "system")]
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "console_audit")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub actor_kind: ActorKind,
    /// `None` for `cli` and `system`.
    pub actor_id: Option<i64>,
    pub action: String,
    pub target_type: String,
    /// Empty when the action has no single target.
    pub target_id: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub ip: Option<String>,
    pub created_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
