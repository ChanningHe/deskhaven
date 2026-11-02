//! Dual-backend test helpers (feature `testing`).
//!
//! Every test body runs on a fresh SQLite file and, when `DESKHAVEN_TEST_PG_URL` is set, on
//! PostgreSQL in a private, uniquely named schema that is dropped afterwards (a panicking test
//! leaves its `dh_test_*` schema behind; harmless in a throwaway database).

use std::fmt;
use std::io::Write as _;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sea_orm::{ConnectionTrait, DatabaseConnection};
use tempfile::TempDir;

use crate::{DbOptions, connect, migrate};

/// Env var holding an admin-capable PostgreSQL URL for tests.
pub const PG_URL_ENV: &str = "DESKHAVEN_TEST_PG_URL";

const SCHEMA_PREFIX: &str = "dh_test";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Sqlite,
    Postgres,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        })
    }
}

/// A fresh, isolated database for one test run.
pub struct TestDb {
    pub conn: DatabaseConnection,
    pub backend: Backend,
    pub url: String,
    guard: Guard,
}

enum Guard {
    /// Keeps the temp dir (and the database file) alive until the test ends.
    Sqlite { _dir: TempDir },
    Postgres {
        admin: DatabaseConnection,
        schema: String,
    },
}

/// Runs `test` against an empty database on every available backend.
/// Each database is created right before its run, so a failure never leaks the next one.
pub async fn each_backend<F: AsyncFn(&TestDb)>(test: F) {
    let sqlite = sqlite_db().await;
    test(&sqlite).await;
    sqlite.cleanup().await;

    let Some(pg_url) = pg_url() else { return };
    let pg = postgres_db(&pg_url).await;
    test(&pg).await;
    pg.cleanup().await;
}

/// Like [`each_backend`], with all migrations applied.
pub async fn each_migrated_backend<F: AsyncFn(&TestDb)>(test: F) {
    each_backend(async |db: &TestDb| {
        must(migrate(&db.conn).await, "migrate");
        test(db).await;
    })
    .await;
}

fn pg_url() -> Option<String> {
    let url = std::env::var(PG_URL_ENV).ok().filter(|u| !u.is_empty());
    if url.is_none() {
        warn_pg_skipped();
    }
    url
}

async fn sqlite_db() -> TestDb {
    let dir = must(tempfile::tempdir(), "create temp dir");
    let url = format!("sqlite://{}", dir.path().join("test.db").display());
    let conn = must(
        connect(&DbOptions::new(url.clone())).await,
        "connect sqlite",
    );
    TestDb {
        conn,
        backend: Backend::Sqlite,
        url,
        guard: Guard::Sqlite { _dir: dir },
    }
}

async fn postgres_db(base_url: &str) -> TestDb {
    let admin = must(connect(&DbOptions::new(base_url)).await, "connect postgres");
    let schema = unique_schema_name();
    let create = format!("CREATE SCHEMA \"{schema}\"");
    must(admin.execute_unprepared(&create).await, "create schema");
    let url = with_search_path(base_url, &schema);
    let conn = must(
        connect(&DbOptions::new(url.clone())).await,
        "connect schema",
    );
    TestDb {
        conn,
        backend: Backend::Postgres,
        url,
        guard: Guard::Postgres { admin, schema },
    }
}

impl TestDb {
    async fn cleanup(self) {
        let _ = self.conn.close().await;
        if let Guard::Postgres { admin, schema } = self.guard {
            let drop = format!("DROP SCHEMA \"{schema}\" CASCADE");
            must(admin.execute_unprepared(&drop).await, "drop schema");
            let _ = admin.close().await;
        }
    }
}

fn unique_schema_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{SCHEMA_PREFIX}_{}_{n}_{nanos}", std::process::id())
}

/// Appends a startup `search_path` option so every pooled connection uses `schema`.
fn with_search_path(url: &str, schema: &str) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}options[search_path]={schema}")
}

/// Printed straight to stderr (bypassing libtest capture) so the skip is always visible.
fn warn_pg_skipped() {
    static WARNED: Once = Once::new();
    WARNED.call_once(|| {
        let _ = writeln!(
            std::io::stderr(),
            "warning: {PG_URL_ENV} not set; PostgreSQL DB tests skipped (SQLite only)"
        );
    });
}

/// Test-setup failures abort the test with context.
#[allow(clippy::panic)]
fn must<T, E: fmt::Debug>(res: Result<T, E>, what: &str) -> T {
    res.unwrap_or_else(|e| panic!("test db setup failed ({what}): {e:?}"))
}
