//! Background jobs (M2-20): audit retention and expiry cleanup on a manual clock, in
//! batches that commit one by one. Every test runs on SQLite and PostgreSQL.

mod batches;
mod expiry;
mod retention;
mod support;
