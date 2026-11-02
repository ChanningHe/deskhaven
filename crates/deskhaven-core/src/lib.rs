//! Domain services. No HTTP and no protobuf types live here.

pub mod addressbooks;
pub mod auth;
pub mod clock;
pub mod console_audit;
pub mod crypto;
pub mod devices;
pub mod groups;
pub mod keys;
pub mod login_events;
pub mod oidc;
pub mod pagination;
pub mod sessions;
pub mod settings;
pub mod state;
pub mod strategies;
mod text;
pub mod tfa;
pub mod users;
