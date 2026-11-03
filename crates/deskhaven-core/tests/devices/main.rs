//! Device service (M2-12): id + uuid verification, sysinfo upsert, deployment presets,
//! device and device group deletion; `--deploy` and `--assign` (M2-19). Every test runs on
//! SQLite and PostgreSQL.

mod admin;
mod assign;
mod assign_failures;
mod deletion;
mod deploy;
mod deploy_guards;
mod flush;
mod heartbeat;
mod presets;
mod support;
mod sysinfo;
mod verify;
