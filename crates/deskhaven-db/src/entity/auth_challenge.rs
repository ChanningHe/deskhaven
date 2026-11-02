//! `auth_challenges`: pending second login step (email code or TOTP).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Kind {
    #[sea_orm(string_value = "email_code")]
    EmailCode,
    #[sea_orm(string_value = "totp")]
    Totp,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_challenges")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// SHA-256 of the 256-bit random secret returned to the client.
    pub secret_hash: String,
    pub user_id: i64,
    pub kind: Kind,
    /// Emailed code, hashed with HMAC(k_hmac, id || code) by the service (a 6-digit code is
    /// brute-forceable under plain SHA-256); `Some` exactly for `email_code`.
    pub code_hash: Option<String>,
    pub attempts: i32,
    /// JSON: rustdesk_id, uuid, device_info.
    pub context: String,
    pub created_at: TimestampMs,
    pub expires_at: TimestampMs,
    /// Single-use marker, set by `UPDATE ... WHERE consumed_at IS NULL`.
    pub consumed_at: Option<TimestampMs>,
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
