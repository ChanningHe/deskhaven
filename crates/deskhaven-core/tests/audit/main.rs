//! Audit ingest (M2-15): `/api/audit/{conn,file,alarm}` records merged into
//! `connection_sessions`, de-duplicated by nonce, refused for unknown devices. Every test runs
//! on SQLite and PostgreSQL.

mod ingest;
mod merge;
mod notes;
mod support;
