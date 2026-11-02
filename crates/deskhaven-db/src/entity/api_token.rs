//! `api_tokens`: automation tokens (`dhk_` prefix); only the SHA-256 of the token is stored.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "api_tokens")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// Leading characters of the token, shown in the UI to tell tokens apart.
    pub token_prefix: String,
    pub token_hash: String,
    /// JSON array of scopes.
    pub scopes: String,
    /// `None` once the creating user is deleted.
    pub created_by: Option<i64>,
    pub created_at: TimestampMs,
    pub expires_at: Option<TimestampMs>,
    pub last_used_at: Option<TimestampMs>,
    pub revoked_at: Option<TimestampMs>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::CreatedBy",
        to = "super::user::Column::Id",
        on_delete = "SetNull"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
