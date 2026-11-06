//! Domain services. No HTTP and no protobuf types live here.

pub mod accessible;
pub mod addressbooks;
pub mod api_tokens;
pub mod audit;
pub mod audit_query;
pub mod auth;
pub mod client_config;
pub mod clock;
mod commit_hooks;
pub mod console_audit;
pub mod crypto;
pub mod devices;
pub mod groups;
pub mod jobs;
pub mod keys;
pub mod ldap;
pub mod login_events;
pub mod oidc;
pub mod pagination;
pub mod sessions;
pub mod settings;
pub mod setup;
pub mod state;
pub mod strategies;
mod text;
pub mod tfa;
pub mod users;
