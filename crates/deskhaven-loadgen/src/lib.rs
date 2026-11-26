//! Spawns `deskhaven serve`, then simulates N RustDesk devices the way the official client
//! behaves:
//!
//! - **Signal**: a fresh install sends `RegisterPk`; once the key is confirmed it sends
//!   `RegisterPeer` every 12 s and `RegisterPk` again whenever the answer asks for the key.
//!   One UDP socket per device.
//! - **Client API**: a 3 s tick; sysinfo first when not yet uploaded (or when a heartbeat
//!   reply asks for it), then a heartbeat once 15 s passed since the last one, or every tick
//!   while a connection is live. A new TCP connection per call.
//! - **Audit**: now and then a connection opens; its `new`, authorized and `close` records go
//!   out on consecutive ticks.
//!
//! Each device has its own loopback source address, so per-IP limits see N clients.

pub mod dbstats;
pub mod device;
pub mod error;
pub mod fdlimit;
pub mod http;
pub mod machine;
pub mod plan;
pub mod procfs;
pub mod rendezvous;
pub mod report;
pub mod requests;
pub mod run;
pub mod sampler;
pub mod server;
pub mod stats;

pub use error::LoadgenError;
pub use report::Report;
pub use run::{Config, run};
