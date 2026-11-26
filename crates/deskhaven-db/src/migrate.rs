use std::collections::BTreeSet;

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, DbErr, EntityTrait,
};
use sea_orm_migration::{MigratorTrait, SchemaManager, seaql_migrations};

use crate::migration::Migrator;
use crate::{DbError, begin_write};

/// Transaction-scoped PostgreSQL advisory lock key serializing concurrent migrators
/// (ASCII "dhmigrat").
const PG_MIGRATION_LOCK_KEY: i64 = 0x6468_6d69_6772_6174;

pub async fn migrate(conn: &DatabaseConnection) -> Result<(), DbError> {
    migrate_with::<Migrator>(conn).await
}

/// One transaction: every pending migration and its version record commit, or nothing does.
/// Fails if the database has versions unknown to `M` (newer binary). Concurrent starters
/// serialize (SQLite `BEGIN IMMEDIATE`, PostgreSQL `pg_advisory_xact_lock`).
pub async fn migrate_with<M: MigratorTrait>(conn: &DatabaseConnection) -> Result<(), DbError> {
    let txn = begin_write(conn).await.map_err(DbError::Migration)?;
    // On error the uncommitted transaction is rolled back when dropped.
    lock_migrations(&txn).await.map_err(DbError::Migration)?;
    M::up(&txn, None).await.map_err(DbError::Migration)?;
    txn.commit().await.map_err(DbError::Migration)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatus {
    pub pending: Vec<String>,
    /// Applied, but unknown to this binary: the database was migrated by a newer version.
    pub unknown: Vec<String>,
}

impl MigrationStatus {
    pub fn is_current(&self) -> bool {
        self.pending.is_empty() && self.unknown.is_empty()
    }
}

/// Read-only: never creates the migration table (a fresh database reports every
/// migration as pending), so it works on a read-only connection.
pub async fn migration_status(conn: &DatabaseConnection) -> Result<MigrationStatus, DbError> {
    let applied = applied_versions(conn)
        .await
        .map_err(DbError::MigrationStatus)?;
    let known: Vec<String> = Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect();
    let pending = known
        .iter()
        .filter(|name| !applied.contains(*name))
        .cloned()
        .collect();
    let unknown = applied
        .into_iter()
        .filter(|version| !known.contains(version))
        .collect();
    Ok(MigrationStatus { pending, unknown })
}

async fn applied_versions(conn: &DatabaseConnection) -> Result<BTreeSet<String>, DbErr> {
    let table = Migrator::migration_table_name().to_string();
    if !SchemaManager::new(conn).has_table(&table).await? {
        return Ok(BTreeSet::new());
    }
    let rows = seaql_migrations::Entity::find().all(conn).await?;
    Ok(rows.into_iter().map(|row| row.version).collect())
}

async fn lock_migrations(txn: &DatabaseTransaction) -> Result<(), DbErr> {
    if txn.get_database_backend() == DbBackend::Postgres {
        let sql = format!("SELECT pg_advisory_xact_lock({PG_MIGRATION_LOCK_KEY})");
        txn.execute_unprepared(&sql).await?;
    }
    Ok(())
}
