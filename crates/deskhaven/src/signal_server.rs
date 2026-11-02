//! The embedded signal server (hbbs, `deskhaven-signal`) in `deskhaven serve` (M1-06).
//!
//! - Configuration ([`base_config`]): `[signal] port` from the bootstrap, the hbbs key pair
//!   converted to sodiumoxide's layout, runtime settings from the settings snapshot at each
//!   (re)start. Listeners bind all interfaces (IPv6 dual-stack, else IPv4) like upstream
//!   without `--bind`: the client API address is no guide, it is often loopback behind a
//!   reverse proxy, while the signal ports (raw TCP/UDP) must face clients directly. The
//!   websocket port honours `X-Forwarded-For` only from `security.trusted_proxies`, by the
//!   HTTP API's rule (`deskhaven_http::TrustedProxies`).
//! - Registrations persist through [`SignalPeerStore`](crate::signal_store::SignalPeerStore);
//!   policy is [`AllowAll`](deskhaven_signal::AllowAll) until M5, wrapped in [`CoreControl`],
//!   which feeds the signal server's liveness notifications into the device presence table.
//! - `signal.relay_servers`, `signal.always_use_relay` and `signal.require_kx_v1` follow the
//!   settings at runtime (`relay_sync`).
//! - [`start`] fails startup if the first attempt cannot serve (port taken, peer registry not
//!   loadable); afterwards a supervisor restarts the server with backoff whenever it fails
//!   (`supervisor`), never leaving the process without a signal server silently.

mod config;
mod control;
mod relay_sync;
mod supervisor;

pub use config::{SignalSettings, base_config, secret_key};
pub use control::CoreControl;
pub use supervisor::{Backoff, Launcher, RunningSignal, SignalStatus, spawn_supervised, start};

#[cfg(test)]
mod deletion_tests;
#[cfg(test)]
mod kx_tests;
#[cfg(test)]
mod tests;
