//! PostgreSQL: `pg_stat_user_tables` of the connection's schema plus the database's
//! `xact_commit`. Backends report counters when a transaction ends, at most once a second
//! (an idle one within about 10 s), so a snapshot can miss the last seconds' writes; the
//! window's bound allows for one flush.
//! SQLite has no such statistics: [`DbProbe::install`] adds triggers that bump a per-table
//! counter inside the statement that fired them, so the count is exact and rolls back with a
//! failed statement. The extra counter update makes the SQLite figures slightly pessimistic.

use std::collections::BTreeMap;

use deskhaven_db::DbOptions;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};

use crate::LoadgenError;

const COUNTER_TABLE: &str = "loadgen_row_writes";
const OPS: [(&str, &str); 3] = [("INSERT", "ins"), ("UPDATE", "upd"), ("DELETE", "del")];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowWrites {
    pub inserted: i64,
    pub updated: i64,
    pub deleted: i64,
}

impl RowWrites {
    pub fn total(&self) -> i64 {
        self.inserted + self.updated + self.deleted
    }

    fn since(&self, earlier: &Self) -> Self {
        Self {
            inserted: self.inserted - earlier.inserted,
            updated: self.updated - earlier.updated,
            deleted: self.deleted - earlier.deleted,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub tables: BTreeMap<String, RowWrites>,
    /// PostgreSQL only: committed transactions of the database.
    pub commits: Option<i64>,
}

impl Snapshot {
    /// Tables without writes are left out.
    pub fn since(&self, earlier: &Self) -> Self {
        let tables = self
            .tables
            .iter()
            .map(|(name, now)| {
                let before = earlier.tables.get(name).copied().unwrap_or_default();
                (name.clone(), now.since(&before))
            })
            .filter(|(_, delta)| delta.total() != 0)
            .collect();
        let commits = self.commits.zip(earlier.commits).map(|(a, b)| a - b);
        Self { tables, commits }
    }

    pub fn rows_written(&self) -> i64 {
        self.tables.values().map(RowWrites::total).sum()
    }

    pub fn table(&self, name: &str) -> RowWrites {
        self.tables.get(name).copied().unwrap_or_default()
    }
}

pub struct DbProbe {
    conn: DatabaseConnection,
}

impl DbProbe {
    pub async fn open(url: &str) -> Result<Self, LoadgenError> {
        let mut options = DbOptions::new(url);
        options.max_connections = 1;
        let conn = deskhaven_db::connect(&options).await?;
        Ok(Self { conn })
    }

    pub fn backend(&self) -> DbBackend {
        self.conn.get_database_backend()
    }

    pub async fn install(&self) -> Result<(), LoadgenError> {
        if self.backend() != DbBackend::Sqlite {
            return Ok(());
        }
        let tables = self.sqlite_tables().await?;
        let txn = self.conn.begin().await?;
        txn.execute_unprepared(&format!(
            "CREATE TABLE IF NOT EXISTS {COUNTER_TABLE} (tbl TEXT NOT NULL, op TEXT NOT NULL, \
             n INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (tbl, op))"
        ))
        .await?;
        for table in &tables {
            for statement in trigger_statements(table) {
                txn.execute_unprepared(&statement).await?;
            }
        }
        txn.commit().await?;
        tracing::info!(tables = tables.len(), "SQLite write counters installed");
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<Snapshot, LoadgenError> {
        match self.backend() {
            DbBackend::Sqlite => self.sqlite_snapshot().await,
            _ => self.postgres_snapshot().await,
        }
    }

    /// Others are skipped, never quoted.
    async fn sqlite_tables(&self) -> Result<Vec<String>, LoadgenError> {
        let rows = self
            .conn
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            ))
            .await?;
        let names = rows
            .iter()
            .map(|row| row.try_get::<String>("", "name"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(names
            .into_iter()
            .filter(|name| name != COUNTER_TABLE && is_plain_identifier(name))
            .collect())
    }

    async fn sqlite_snapshot(&self) -> Result<Snapshot, LoadgenError> {
        let sql = format!("SELECT tbl, op, n FROM {COUNTER_TABLE}");
        let rows = self
            .conn
            .query_all_raw(Statement::from_string(DbBackend::Sqlite, sql))
            .await?;
        let mut tables: BTreeMap<String, RowWrites> = BTreeMap::new();
        for row in rows {
            let table: String = row.try_get("", "tbl")?;
            let op: String = row.try_get("", "op")?;
            let n: i64 = row.try_get("", "n")?;
            let entry = tables.entry(table).or_default();
            match op.as_str() {
                "ins" => entry.inserted = n,
                "upd" => entry.updated = n,
                _ => entry.deleted = n,
            }
        }
        Ok(Snapshot {
            tables,
            commits: None,
        })
    }

    async fn postgres_snapshot(&self) -> Result<Snapshot, LoadgenError> {
        let rows = self
            .conn
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT relname::text AS relname, n_tup_ins, n_tup_upd, n_tup_del \
                 FROM pg_stat_user_tables WHERE schemaname = current_schema()",
            ))
            .await?;
        let mut tables = BTreeMap::new();
        for row in rows {
            let counts = RowWrites {
                inserted: row.try_get("", "n_tup_ins")?,
                updated: row.try_get("", "n_tup_upd")?,
                deleted: row.try_get("", "n_tup_del")?,
            };
            tables.insert(row.try_get::<String>("", "relname")?, counts);
        }
        let commits = self
            .conn
            .query_one_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT xact_commit FROM pg_stat_database WHERE datname = current_database()",
            ))
            .await?
            .map(|row| row.try_get::<i64>("", "xact_commit"))
            .transpose()?;
        Ok(Snapshot { tables, commits })
    }
}

fn trigger_statements(table: &str) -> Vec<String> {
    let seed = format!(
        "INSERT OR IGNORE INTO {COUNTER_TABLE} (tbl, op, n) VALUES \
         ('{table}', 'ins', 0), ('{table}', 'upd', 0), ('{table}', 'del', 0)"
    );
    let triggers = OPS.iter().map(|(event, op)| {
        format!(
            "CREATE TRIGGER IF NOT EXISTS loadgen_{table}_{op} AFTER {event} ON {table} \
             BEGIN UPDATE {COUNTER_TABLE} SET n = n + 1 WHERE tbl = '{table}' AND op = '{op}'; END"
        )
    });
    std::iter::once(seed).chain(triggers).collect()
}

/// ASCII letters, digits and `_`, not starting with a digit: safe to splice into SQL.
fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests;
