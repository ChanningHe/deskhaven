//! Test support only. Production crates must never depend on this crate: a server that
//! deserializes its own responses with its own types proves nothing.

pub mod ab_seeds;
pub mod app;
pub mod check;
pub mod device_seeds;
pub mod fixture;
pub mod group_seeds;
pub mod mirror;
pub mod requests;
pub mod runner;
pub mod seeds;
pub mod shapes;
pub mod token_seeds;
