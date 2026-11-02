//! Startup migration runner.

use sea_orm::{ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr};
use sea_orm_migration::MigratorTrait;

use crate::migration::Migrator;
use crate::{DbError, begin_write};

/// Transaction-scoped PostgreSQL advisory lock key serializing concurrent migrators
/// (ASCII "dhmigrat").
const PG_MIGRATION_LOCK_KEY: i64 = 0x6468_6d69_6772_6174;

/// Applies all pending migrations of the built-in [`Migrator`].
pub async fn migrate(conn: &DatabaseConnection) -> Result<(), DbError> {
    migrate_with::<Migrator>(conn).await
}

/// Applies all pending migrations of `M` in a single transaction: either every pending
/// migration and its version record is committed, or nothing is (both backends support
/// transactional DDL). Fails if the database has versions unknown to `M` (newer binary).
///
/// Concurrent starters serialize: SQLite via `BEGIN IMMEDIATE` (see [`begin_write`]),
/// PostgreSQL via `pg_advisory_xact_lock`; the second one then finds nothing pending.
pub async fn migrate_with<M: MigratorTrait>(conn: &DatabaseConnection) -> Result<(), DbError> {
    let txn = begin_write(conn).await.map_err(DbError::Migration)?;
    // On error the uncommitted transaction is rolled back when dropped.
    lock_migrations(&txn).await.map_err(DbError::Migration)?;
    M::up(&txn, None).await.map_err(DbError::Migration)?;
    txn.commit().await.map_err(DbError::Migration)
}

async fn lock_migrations(txn: &DatabaseTransaction) -> Result<(), DbErr> {
    if txn.get_database_backend() == DbBackend::Postgres {
        let sql = format!("SELECT pg_advisory_xact_lock({PG_MIGRATION_LOCK_KEY})");
        txn.execute_unprepared(&sql).await?;
    }
    Ok(())
}
