//! Startup migration runner.

use sea_orm::{DatabaseConnection, TransactionTrait};
use sea_orm_migration::MigratorTrait;

use crate::DbError;
use crate::migration::Migrator;

/// Applies all pending migrations of the built-in [`Migrator`].
pub async fn migrate(conn: &DatabaseConnection) -> Result<(), DbError> {
    migrate_with::<Migrator>(conn).await
}

/// Applies all pending migrations of `M` in a single transaction: either every pending
/// migration and its version record is committed, or nothing is (both backends support
/// transactional DDL). Fails if the database has versions unknown to `M` (newer binary).
pub async fn migrate_with<M: MigratorTrait>(conn: &DatabaseConnection) -> Result<(), DbError> {
    let txn = conn.begin().await.map_err(DbError::Migration)?;
    // On error the uncommitted transaction is rolled back when dropped.
    M::up(&txn, None).await.map_err(DbError::Migration)?;
    txn.commit().await.map_err(DbError::Migration)
}
