//! Audit ingest (M2-15): `/api/audit/{conn,file,alarm}` records merged into
//! `connection_sessions`, de-duplicated by nonce, refused for unknown devices, rate limited
//! per device. Every test runs on SQLite and PostgreSQL.

mod info;
mod ingest;
mod limits;
mod merge;
mod notes;
mod reuse;
mod support;
