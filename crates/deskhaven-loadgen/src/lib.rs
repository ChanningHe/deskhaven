//! Load generator for deskhaven (M4-04; 09-testing §1 load row, NFRs of 01-prd §4).
//!
//! Spawns `deskhaven serve`, then simulates N RustDesk devices the way the official client
//! behaves (client master `23e5bb4`):
//!
//! - **Signal** (`src/rendezvous_mediator.rs`): a fresh install sends `RegisterPk`; once the
//!   key is confirmed it sends `RegisterPeer` every `REG_INTERVAL` (12 s) and `RegisterPk`
//!   again whenever the answer asks for the key (`request_pk`). One UDP socket per device.
//! - **Client API** (`src/hbbs_http/sync.rs`, 03 §2): a 3 s tick; sysinfo first when not yet
//!   uploaded (or when a heartbeat reply asks for it), then a heartbeat once 15 s passed since
//!   the last one, or every tick while a connection is live. A new TCP connection per call
//!   (`post_request` builds a fresh client).
//! - **Audit** (`src/server/connection.rs`, 03 §2.3): now and then a connection opens; its
//!   `new`, authorized and `close` records go out on consecutive ticks.
//!
//! Each device has its own loopback source address (127.0.0.0/8 routes to `lo` on Linux), so
//! per-IP limits see N clients, as in a real fleet.
//!
//! Measured: client-observed latency per request kind; CPU and RSS of the server process only
//! (`/proc/<pid>`); database row writes per table (PostgreSQL `pg_stat_user_tables`, SQLite
//! counter triggers, see [`dbstats`]).

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
