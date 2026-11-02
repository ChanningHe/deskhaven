//! SeaORM entities. Conventions (04-data-model §1): time columns are
//! [`TimestampMs`](crate::time::TimestampMs), JSON is stored as `TEXT` (serde in the app layer),
//! enums are `TEXT` guarded by a CHECK constraint and mapped to Rust enums.

pub mod address_book;
pub mod address_book_peer;
pub mod address_book_share;
pub mod address_book_tag;
pub mod alarm;
pub mod api_token;
pub mod audit_nonce;
pub mod auth_challenge;
pub mod auth_lockout;
pub mod connection_session;
pub mod console_audit;
pub mod device;
pub mod device_group;
pub mod file_transfer;
pub mod group;
pub mod group_member;
pub mod ldap_directory;
pub mod login_event;
pub mod managed_by;
pub mod oidc_flow;
pub mod oidc_provider;
pub mod recovery_code;
pub mod session;
pub mod setting;
pub mod setting_history;
pub mod setup_token;
pub mod strategy;
pub mod strategy_assignment;
pub mod user;
pub mod user_identity;
