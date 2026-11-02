// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
//! Rendezvous (signal) server: vendored hard fork of upstream hbbs (see UPSTREAM, CHANGES.md),
//! embedded as a library: build a [`SignalConfig`], then [`run`] it (or [`SignalServer::run`]
//! when relay settings must change at runtime through a [`SignalHandle`]) on the caller's tokio
//! runtime until a `CancellationToken` is cancelled.
//!
//! The vendored modules are exempted from the clippy lints they trip so they can stay close to
//! upstream for now; remove an entry as soon as the code no longer needs it. Non-vendored
//! modules get the full workspace lints.

// Vendored upstream code; clean up incrementally.
#[allow(
    clippy::unwrap_used,
    clippy::unused_io_amount,
    clippy::unnecessary_unwrap,
    clippy::unnecessary_get_then_check,
    clippy::too_many_arguments,
    clippy::result_large_err,
    clippy::needless_lifetimes,
    clippy::manual_div_ceil,
    clippy::collapsible_match,
    clippy::useless_borrows_in_formatting
)]
mod rendezvous_server;
// Vendored upstream code; trips no clippy lints.
pub mod common;
mod database;
// Vendored upstream code; clean up incrementally.
#[allow(clippy::unwrap_used)]
mod peer;
mod server;
mod tasks;

pub use server::{
    run, SignalConfig, SignalError, SignalHandle, SignalServer, DEFAULT_PUNCH_REQ_MAX,
};
