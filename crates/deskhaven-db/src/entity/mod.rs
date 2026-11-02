//! SeaORM entities. Conventions (04-data-model §1): time columns are
//! [`TimestampMs`](crate::time::TimestampMs), JSON is stored as `TEXT` (serde in the app layer),
//! enums are `TEXT` guarded by a CHECK constraint and mapped to Rust enums.

pub mod api_token;
pub mod auth_challenge;
pub mod group;
pub mod group_member;
pub mod ldap_directory;
pub mod managed_by;
pub mod oidc_flow;
pub mod oidc_provider;
pub mod recovery_code;
pub mod session;
pub mod setting;
pub mod setting_history;
pub mod setup_token;
pub mod user;
pub mod user_identity;
