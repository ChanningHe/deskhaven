//! `oidc_providers`: configured OpenID Connect identity providers.

use sea_orm::entity::prelude::*;

use super::managed_by::ManagedBy;
use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oidc_providers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Unique short name; the client's `op` value.
    pub name: String,
    pub display_name: String,
    pub icon: Option<String>,
    pub issuer: String,
    pub client_id: String,
    /// Encrypted client secret (`v1:<key_id>:<nonce>:<ct>`); `None` for public (PKCE-only) clients.
    pub client_secret_enc: Option<String>,
    /// Space-separated scopes.
    pub scopes: String,
    pub auto_provision: bool,
    pub group_claim: Option<String>,
    pub admin_group: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::user_identity::Entity")]
    UserIdentity,
    #[sea_orm(has_many = "super::oidc_flow::Entity")]
    OidcFlow,
}

impl Related<super::user_identity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::UserIdentity.def()
    }
}

impl Related<super::oidc_flow::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OidcFlow.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
