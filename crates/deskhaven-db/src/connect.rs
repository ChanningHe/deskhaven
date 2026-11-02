//! Connection factory for `sqlite:` and `postgres:` URLs.
//!
//! PostgreSQL TLS (`sslmode=require` etc.) uses rustls (ring) with the bundled Mozilla
//! (webpki) root store, not the system store: a server certificate from an internal CA
//! needs `sslrootcert=/path/to/ca.pem` in the URL.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use sea_orm::sqlx::postgres::PgConnectOptions;
use sea_orm::sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};

use crate::DbError;
use crate::redact::redact_url;

/// How long a SQLite writer waits on a locked database before failing.
pub const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connection settings (mirrors the bootstrap `[database]` section). Build with
/// [`DbOptions::new`], then adjust fields. `Debug` masks credentials in the URL.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DbOptions {
    pub url: String,
    pub max_connections: u32,
    /// Upper bound for opening (and acquiring) a pooled connection.
    pub connect_timeout: Duration,
    /// Inspect without writing (`config check`): SQLite opens read-only and never creates
    /// the file or changes its journal mode; PostgreSQL sessions default to read-only
    /// transactions (`default_transaction_read_only`). PgBouncer in transaction-pool mode
    /// rejects that startup parameter unless listed in `ignore_startup_parameters`, so a
    /// read-only connection fails there while a read-write one works.
    pub read_only: bool,
}

impl DbOptions {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_only: false,
        }
    }
}

impl fmt::Debug for DbOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DbOptions")
            .field("url", &redact_url(&self.url))
            .field("max_connections", &self.max_connections)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_only", &self.read_only)
            .finish()
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
/// Applied after URL parsing, so these always override the same settings given in the URL
/// (`mode`, `journal_mode`, `synchronous`, `busy_timeout`, `foreign_keys` cannot be changed).
fn sqlite_tuning(opts: SqliteConnectOptions) -> SqliteConnectOptions {
    opts.create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
        .foreign_keys(true)
}

/// SQLite inspection: an existing file only, no pragma that writes (no `journal_mode`).
///
/// A read-only connection to a WAL database still creates missing `-wal`/`-shm` files
/// (SQLite >= 3.22). Created by another user (`sudo deskhaven config check`), they would
/// make the database read-only for the server. Without a live writer (neither side file
/// exists) the file is opened `immutable` instead, which never creates them; with a live
/// writer both files exist and a plain read-only connection sees its committed frames.
fn sqlite_read_only(opts: SqliteConnectOptions) -> SqliteConnectOptions {
    let is_idle = !SQLITE_SIDE_FILES
        .iter()
        .any(|suffix| side_file(opts.get_filename(), suffix).exists());
    opts.read_only(true)
        .immutable(is_idle)
        .create_if_missing(false)
        .busy_timeout(SQLITE_BUSY_TIMEOUT)
}

/// Files SQLite keeps beside a WAL-mode database while it is open.
const SQLITE_SIDE_FILES: [&str; 2] = ["-wal", "-shm"];

fn side_file(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn postgres_read_only(opts: PgConnectOptions) -> PgConnectOptions {
    opts.options([("default_transaction_read_only", "on")])
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
    match (scheme, opts.read_only) {
        (Scheme::Sqlite, false) => connect_opts.map_sqlx_sqlite_opts(sqlite_tuning),
        (Scheme::Sqlite, true) => connect_opts.map_sqlx_sqlite_opts(sqlite_read_only),
        (Scheme::Postgres, false) => &mut connect_opts,
        (Scheme::Postgres, true) => connect_opts.map_sqlx_postgres_opts(postgres_read_only),
    };
    Database::connect(connect_opts)
        .await
        .map_err(DbError::Connect)
}

/// The database file of a `sqlite:` URL (percent-decoded, query stripped); `None` for
/// other backends or an unparsable URL.
pub fn sqlite_file(url: &str) -> Option<PathBuf> {
    if scheme_of(url).ok()? != Scheme::Sqlite {
        return None;
    }
    SqliteConnectOptions::from_str(url)
        .ok()
        .map(|opts| opts.get_filename().to_path_buf())
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
