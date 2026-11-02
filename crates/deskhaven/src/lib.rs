//! deskhaven server: command-line interface, startup and shutdown wiring.
//!
//! The binary (`main.rs`) only parses arguments and dispatches here; keeping
//! the wiring in a library makes it unit-testable.

pub mod check;
pub mod cli;
pub mod config_cmd;
pub mod data_dir;
pub mod database;
pub mod http;
pub mod instance_lock;
pub mod key_cmd;
pub mod log_safe;
pub mod logging;
pub mod serve;
pub mod setup_link;
pub mod shutdown;
pub mod signal_server;
pub mod signal_store;
pub mod startup;
pub mod user_cmd;

use anyhow::anyhow;

/// Makes ring the process-wide rustls provider (02 §5.1). Must run before
/// anything touches TLS (PostgreSQL connections included).
pub fn install_crypto_provider() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("a TLS crypto provider was already installed"))
}
