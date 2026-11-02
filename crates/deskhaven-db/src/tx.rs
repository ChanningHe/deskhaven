//! Transactions: write (immediate on SQLite) and read (one snapshot).

use sea_orm::{
    AccessMode, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, IsolationLevel,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

/// Begins a transaction that will write. **Use it for every read-then-write transaction.**
///
/// SQLite: issues `BEGIN IMMEDIATE`, taking the write lock up front. A plain (deferred)
/// `BEGIN` that reads and then writes fails at once with `SQLITE_BUSY` (`database is
/// locked`) when another writer committed in between: in WAL mode the stale read snapshot
/// cannot be upgraded and `busy_timeout` does not apply. With `IMMEDIATE`, concurrent
/// writers instead wait (up to [`SQLITE_BUSY_TIMEOUT`](crate::SQLITE_BUSY_TIMEOUT)) and
/// run one after another; readers are never blocked (WAL).
///
/// PostgreSQL: a plain `BEGIN` (READ COMMITTED). Guard read-modify-write with row locks
/// (`SELECT ... FOR UPDATE`) or optimistic `version` checks.
pub async fn begin_write(conn: &DatabaseConnection) -> Result<DatabaseTransaction, DbErr> {
    let options = match conn.get_database_backend() {
        DbBackend::Sqlite => TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..TransactionOptions::default()
        },
        _ => TransactionOptions::default(),
    };
    conn.begin_with_options(options).await
}

/// Begins a read-only transaction whose queries all see one snapshot, for reads that join
/// several tables by hand.
///
/// SQLite: a plain deferred `BEGIN`; in WAL mode the first read fixes the snapshot.
/// PostgreSQL: `REPEATABLE READ, READ ONLY` (the default READ COMMITTED would let each
/// query see a different state). Finish with `commit` (nothing was written).
pub async fn begin_read(conn: &DatabaseConnection) -> Result<DatabaseTransaction, DbErr> {
    let options = match conn.get_database_backend() {
        DbBackend::Sqlite => TransactionOptions::default(),
        _ => TransactionOptions {
            isolation_level: Some(IsolationLevel::RepeatableRead),
            access_mode: Some(AccessMode::ReadOnly),
            ..TransactionOptions::default()
        },
    };
    conn.begin_with_options(options).await
}
