//! `users`: accounts of every auth source. `username_norm` / `email_norm` are the
//! lowercased forms the unique indexes are built on; the caller must keep them in sync
//! (the schema CHECKs reject a stale `username_norm`).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum AuthSource {
    #[sea_orm(string_value = "local")]
    Local,
    #[sea_orm(string_value = "ldap")]
    Ldap,
    #[sea_orm(string_value = "oidc")]
    Oidc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Role {
    #[sea_orm(string_value = "owner")]
    Owner,
    #[sea_orm(string_value = "admin")]
    Admin,
    #[sea_orm(string_value = "user")]
    User,
}

/// Maps to the client `status` 1 / 0 / -1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Status {
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "disabled")]
    Disabled,
    #[sea_orm(string_value = "unverified")]
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub username: String,
    pub username_norm: String,
    pub display_name: String,
    pub email: Option<String>,
    pub email_norm: Option<String>,
    pub email_verified: bool,
    /// Argon2id PHC string; `None` unless `auth_source` is `local`.
    pub password_hash: Option<String>,
    pub auth_source: AuthSource,
    pub role: Role,
    pub status: Status,
    pub must_change_password: bool,
    /// Encrypted TOTP seed (`v1:<key_id>:<nonce>:<ct>`).
    pub totp_secret_enc: Option<String>,
    pub totp_enabled_at: Option<TimestampMs>,
    /// Last accepted TOTP time step (replay guard).
    pub totp_last_step: Option<i64>,
    pub note: String,
    pub avatar_path: Option<String>,
    pub last_login_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::group_member::Entity")]
    GroupMember,
    #[sea_orm(has_many = "super::user_identity::Entity")]
    UserIdentity,
    #[sea_orm(has_many = "super::session::Entity")]
    Session,
    #[sea_orm(has_many = "super::recovery_code::Entity")]
    RecoveryCode,
    #[sea_orm(has_many = "super::auth_challenge::Entity")]
    AuthChallenge,
    #[sea_orm(has_many = "super::api_token::Entity")]
    ApiToken,
}

impl Related<super::group_member::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GroupMember.def()
    }
}

impl Related<super::user_identity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserIdentity.def()
    }
}

impl Related<super::session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl Related<super::recovery_code::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RecoveryCode.def()
    }
}

impl Related<super::auth_challenge::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AuthChallenge.def()
    }
}

impl Related<super::api_token::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::ApiToken.def()
    }
}

impl Related<super::group::Entity> for Entity {
    fn to() -> RelationDef {
        super::group_member::Relation::Group.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::group_member::Relation::User.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
