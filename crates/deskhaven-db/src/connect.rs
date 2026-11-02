//! Connection factory for `sqlite:` and `postgres:` URLs.

use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};

use crate::DbError;

/// How long a SQLite writer waits on a locked database before failing.
pub const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connection settings (mirrors the bootstrap `[database]` section).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbOptions {
    pub url: String,
    pub max_connections: u32,
    /// Upper bound for opening (and acquiring) a pooled connection.
    pub connect_timeout: Duration,
}

impl DbOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    Sqlite,
    Postgres,
}

fn scheme_of(url: &str) -> Result<Scheme, DbError> {
    let scheme = url.split_once(':').map_or("", |(s, _)| s);
    match scheme {
        "sqlite" => Ok(Scheme::Sqlite),
        "postgres" | "postgresql" => Ok(Scheme::Postgres),
        other => Err(DbError::UnsupportedUrl {
            scheme: other.to_owned(),
        }),
    }
}

/// SQLite: create the file if missing, WAL, busy timeout, foreign keys on (every connection).
fn sqlite_tuning(opts: SqliteConnectOptions) -> SqliteConnectOptions {
    opts.create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .foreign_keys(true)
}

/// Opens a connection pool. Fails fast if the database is unreachable.
pub async fn connect(opts: &DbOptions) -> Result<DatabaseConnection, DbError> {
    let scheme = scheme_of(&opts.url)?;
    let mut connect_opts = ConnectOptions::new(opts.url.clone());
    connect_opts
        .max_connections(opts.max_connections)
        .connect_timeout(opts.connect_timeout)
        // Per-statement `log` output from sqlx is noise; SeaORM tracing spans remain.
        .sqlx_logging(false);
    if scheme == Scheme::Sqlite {
        connect_opts.map_sqlx_sqlite_opts(sqlite_tuning);
    }
    Database::connect(connect_opts)
        .await
        .map_err(DbError::Connect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_detection() {
        assert_eq!(scheme_of("sqlite://a.db").ok(), Some(Scheme::Sqlite));
        assert_eq!(scheme_of("sqlite::memory:").ok(), Some(Scheme::Sqlite));
        assert_eq!(scheme_of("postgres://h/db").ok(), Some(Scheme::Postgres));
        assert_eq!(scheme_of("postgresql://h/db").ok(), Some(Scheme::Postgres));
        assert!(scheme_of("mysql://h/db").is_err());
        assert!(scheme_of("no-scheme").is_err());
    }
}
