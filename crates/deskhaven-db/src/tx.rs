use sea_orm::{
    AccessMode, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, IsolationLevel,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

/// **Use it for every read-then-write transaction.** SQLite: `BEGIN IMMEDIATE`. A deferred
/// `BEGIN` that reads then writes fails at once with `SQLITE_BUSY` when another writer committed
/// in between (WAL cannot upgrade a stale snapshot; `busy_timeout` does not apply).
/// PostgreSQL: READ COMMITTED; guard read-modify-write with `FOR UPDATE` or `version` checks.
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

/// One snapshot for reads that join several tables by hand. PostgreSQL uses `REPEATABLE READ`:
/// READ COMMITTED would let each query see a different state.
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
