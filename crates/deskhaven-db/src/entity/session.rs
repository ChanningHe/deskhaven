//! `sessions`: opaque-token sessions; only the SHA-256 of the token is stored.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Kind {
    #[sea_orm(string_value = "client")]
    Client,
    #[sea_orm(string_value = "console")]
    Console,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub token_hash: String,
    pub kind: Kind,
    pub user_id: i64,
    /// Client sessions: the device the session is bound to.
    pub rustdesk_id: Option<String>,
    pub device_uuid: Option<String>,
    /// JSON: os, type, name.
    pub device_info: Option<String>,
    pub ip: Option<String>,
    pub created_at: TimestampMs,
    pub last_used_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub revoked_at: Option<TimestampMs>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
        to = "super::user::Column::Id",
        on_delete = "Cascade"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
