//! Device service (M2-12): id + uuid verification, sysinfo upsert, deployment presets,
//! device and device group deletion. Every test runs on SQLite and PostgreSQL.

mod admin;
mod deletion;
mod flush;
mod heartbeat;
mod presets;
mod support;
mod sysinfo;
mod verify;
