//! Client API contract tests (09-testing §2).
//!
//! - [`mirror`]: what the official RustDesk client deserializes.
//! - [`fixture`]: the fixture format (`tests/contract/fixtures/**.json`).
//! - [`runner`]: replays each fixture through the real `client_api` router
//!   and middleware ([`app::TestApp`]) and checks it ([`check`]).
//!
//! Test support only. Production crates must never depend on this crate
//! (`tests/no_production_dependents.rs`): a server that deserializes its own
//! responses with its own types proves nothing.

pub mod ab_seeds;
pub mod app;
pub mod check;
pub mod device_seeds;
pub mod fixture;
pub mod mirror;
pub mod requests;
pub mod runner;
pub mod seeds;
pub mod shapes;
