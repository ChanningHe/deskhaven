//! `login_events`: every login attempt with its method and outcome.
//!
//! `user_id` is NULL for unknown usernames and cleared when the user is deleted;
//! `username_attempted` keeps the audit trail readable either way.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Method {
    #[sea_orm(string_value = "password")]
    Password,
    #[sea_orm(string_value = "totp")]
    Totp,
    #[sea_orm(string_value = "email_code")]
    EmailCode,
    #[sea_orm(string_value = "oidc")]
    Oidc,
    #[sea_orm(string_value = "ldap")]
    Ldap,
    #[sea_orm(string_value = "recovery")]
    Recovery,
}

/// Which kind of client attempted the login.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Client {
    #[sea_orm(string_value = "rustdesk")]
    Rustdesk,
    #[sea_orm(string_value = "console")]
    Console,
    #[sea_orm(string_value = "api")]
    Api,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "login_events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: Option<i64>,
    pub username_attempted: String,
    pub method: Method,
    /// Outcome label chosen by the auth service (e.g. `success`, `failure`, `locked`).
    pub result: String,
    pub client: Client,
    pub rustdesk_id: Option<String>,
    pub ip: Option<String>,
    pub created_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::UserId",
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
