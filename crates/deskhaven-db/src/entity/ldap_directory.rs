//! `ldap_directories`: LDAP directories used for authentication (P0 enables one).

use sea_orm::entity::prelude::*;

use super::managed_by::ManagedBy;
use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "ldap_directories")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    pub url: String,
    pub start_tls: bool,
    pub tls_verify: bool,
    pub bind_dn: Option<String>,
    /// Encrypted service-account password (`v1:<key_id>:<nonce>:<ct>`).
    pub bind_password_enc: Option<String>,
    pub user_base_dn: String,
    pub user_filter: String,
    pub username_attr: String,
    pub email_attr: Option<String>,
    pub display_name_attr: Option<String>,
    pub group_base_dn: Option<String>,
    pub group_filter: Option<String>,
    pub admin_group_dn: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
