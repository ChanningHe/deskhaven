//! Database layer: SeaORM entities, migrations and repositories (SQLite and PostgreSQL).

#[cfg(any(test, feature = "testing"))]
pub mod coded_error;
mod connect;
mod constraint;
pub mod entity;
mod error;
mod migrate;
pub mod migration;
mod redact;
#[cfg(feature = "testing")]
pub mod testing;
pub mod time;
mod transient;
mod tx;

pub use connect::{DbOptions, SQLITE_BUSY_TIMEOUT, connect, sqlite_file};
pub use constraint::is_foreign_key_violation;
pub use error::DbError;
pub use migrate::{MigrationStatus, migrate, migrate_with, migration_status};
pub use transient::{chain_has_transient_conflict, is_transient_conflict};
pub use tx::{begin_read, begin_write};
