//! `user_identities`: a user's account at an OIDC provider, keyed by `(provider, subject)`.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "user_identities")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub user_id: i64,
    pub provider_id: i64,
    /// The provider's stable `sub` claim.
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub created_at: TimestampMs,
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
    #[sea_orm(
        belongs_to = "super::oidc_provider::Entity",
        from = "Column::ProviderId",
        to = "super::oidc_provider::Column::Id",
        on_delete = "Restrict"
    )]
    OidcProvider,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::oidc_provider::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::OidcProvider.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
