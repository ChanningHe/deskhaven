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
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "oidc_flows")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// One-time OAuth `state`.
    pub state: String,
    /// Code the client polls with; `None` for console flows.
    pub client_code: Option<String>,
    pub provider_id: i64,
    /// Encrypted PKCE verifier (`v1:<key_id>:<nonce>:<ct>`).
    pub pkce_verifier_enc: String,
    pub nonce: String,
    pub flow: Flow,
    /// JSON flow context (device binding, return target).
    pub context: String,
    pub created_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub completed_session_id: Option<i64>,
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
        belongs_to = "super::session::Entity",
        from = "Column::CompletedSessionId",
        to = "super::session::Column::Id",
        on_delete = "SetNull"
    )]
    Session,
}

impl Related<super::oidc_provider::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OidcProvider.def()
    }
}

impl Related<super::session::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Session.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
