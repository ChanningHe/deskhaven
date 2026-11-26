//! Listeners bind all interfaces: the client API address is no guide (often loopback behind a
//! reverse proxy), while the signal ports must face clients directly.

mod config;
mod control;
mod relay_sync;
mod supervisor;
mod web_client;

pub use config::{SignalSettings, base_config, secret_key};
pub use control::CoreControl;
pub use supervisor::{Backoff, Launcher, RunningSignal, SignalStatus, spawn_supervised, start};

#[cfg(test)]
mod deletion_tests;
#[cfg(test)]
mod deploy_tests;
#[cfg(test)]
mod kx_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_client_tests;
