//! Rendezvous (signal) server: vendored hard fork of upstream hbbs (see UPSTREAM, CHANGES.md).
//!
//! Vendored upstream code trips the lints below; they are exempted crate-wide so the code can
//! stay close to upstream for now and are to be cleaned up incrementally (remove an entry as
//! soon as the code no longer needs it). New code in this crate must not rely on them.
#![allow(
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
pub use rendezvous_server::*;
pub mod common;
mod database;
mod peer;
