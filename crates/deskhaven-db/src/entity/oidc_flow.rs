//! `oidc_flows`: in-flight OIDC authorization state (never kept in process memory).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Flow {
    #[sea_orm(string_value = "client")]
    Client,
    #[sea_orm(string_value = "console")]
    Console,
    /// A signed-in console user linking an identity to their own account (M4-09).
    #[sea_orm(string_value = "link")]
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oidc_flows")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// SHA-256 of the one-time OAuth `state` (256-bit random).
    pub state_hash: String,
    /// SHA-256 of the code the client polls with; `Some` exactly for client flows.
    pub client_code_hash: Option<String>,
    pub provider_id: i64,
    /// Encrypted PKCE verifier (`v1:<key_id>:<nonce>:<ct>`).
    pub pkce_verifier_enc: String,
    pub nonce: String,
    pub flow: Flow,
    /// JSON flow context (device binding, return target).
    pub context: String,
    pub created_at: TimestampMs,
    pub expires_at: TimestampMs,
    /// Set by the callback on success; the session is minted when the flow is consumed.
    pub completed_user_id: Option<i64>,
    /// Single-use marker, set by `UPDATE ... WHERE consumed_at IS NULL`.
    pub consumed_at: Option<TimestampMs>,
    pub error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::oidc_provider::Entity",
        from = "Column::ProviderId",
        to = "super::oidc_provider::Column::Id",
        on_delete = "Cascade"
    )]
    OidcProvider,
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::CompletedUserId",
        to = "super::user::Column::Id",
        on_delete = "Cascade"
    )]
    User,
}

impl Related<super::oidc_provider::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OidcProvider.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
