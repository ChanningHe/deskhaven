//! Connection factory tests.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

use deskhaven_db::testing::{Backend, TestDb, each_backend};
use deskhaven_db::{DbError, DbOptions, connect};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};

async fn pragma_i64(conn: &DatabaseConnection, name: &str) -> i64 {
    pragma_row(conn, name).await.try_get_by_index(0).unwrap()
}

async fn pragma_text(conn: &DatabaseConnection, name: &str) -> String {
    pragma_row(conn, name).await.try_get_by_index(0).unwrap()
}

async fn pragma_row(conn: &DatabaseConnection, name: &str) -> sea_orm::QueryResult {
    let stmt = Statement::from_string(DbBackend::Sqlite, format!("PRAGMA {name}"));
    conn.query_one_raw(stmt).await.unwrap().unwrap()
}

fn sqlite_options(dir: &tempfile::TempDir, file: &str) -> (DbOptions, std::path::PathBuf) {
    let path = dir.path().join(file);
    let url = format!("sqlite://{}", path.display());
    (DbOptions::new(url), path)
}

#[tokio::test]
async fn sqlite_creates_missing_file_and_applies_pragmas() {
    let dir = tempfile::tempdir().unwrap();
    let (opts, path) = sqlite_options(&dir, "fresh.db");
    assert!(!path.exists());

    let conn = connect(&opts).await.unwrap();

    assert!(path.exists(), "database file must be created");
    assert_eq!(pragma_text(&conn, "journal_mode").await, "wal");
    assert_eq!(pragma_i64(&conn, "foreign_keys").await, 1);
    assert_eq!(pragma_i64(&conn, "busy_timeout").await, 5_000);
}

#[tokio::test]
async fn sqlite_pragmas_apply_to_every_pooled_connection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut opts, _) = sqlite_options(&dir, "pool.db");
    opts.max_connections = 4;
    let conn = connect(&opts).await.unwrap();

    // Each open transaction pins a distinct pooled connection.
    let mut txns = Vec::new();
    for _ in 0..opts.max_connections {
        txns.push(conn.begin().await.unwrap());
    }
    for txn in &txns {
        for (pragma, expected) in [("foreign_keys", 1), ("busy_timeout", 5_000)] {
            let stmt = Statement::from_string(DbBackend::Sqlite, format!("PRAGMA {pragma}"));
            let row = txn.query_one_raw(stmt).await.unwrap().unwrap();
            assert_eq!(
                row.try_get_by_index::<i64>(0).unwrap(),
                expected,
                "{pragma}"
            );
        }
    }
}

#[tokio::test]
async fn sqlite_enforces_foreign_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (opts, _) = sqlite_options(&dir, "fk.db");
    let conn = connect(&opts).await.unwrap();
    conn.execute_unprepared(
        "CREATE TABLE parent (id INTEGER PRIMARY KEY);
         CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES parent(id));",
    )
    .await
    .unwrap();

    let res = conn
        .execute_unprepared("INSERT INTO child (id, parent_id) VALUES (1, 42)")
        .await;

    assert!(res.is_err(), "orphan row must be rejected");
}

#[tokio::test]
async fn unsupported_scheme_is_rejected_without_leaking_credentials() {
    let opts = DbOptions::new("mysql://root:hunter2@localhost/db");

    let err = connect(&opts).await.unwrap_err();

    assert!(matches!(err, DbError::UnsupportedUrl { .. }), "{err:?}");
    let msg = err.to_string();
    assert!(msg.contains("mysql"), "{msg}");
    assert!(!msg.contains("hunter2"), "{msg}");
}

#[tokio::test]
async fn unreachable_postgres_returns_connect_error() {
    let mut opts = DbOptions::new("postgres://nobody@127.0.0.1:1/none");
    opts.connect_timeout = std::time::Duration::from_secs(1);

    let err = connect(&opts).await.unwrap_err();

    assert!(matches!(err, DbError::Connect(_)), "{err:?}");
}

#[test]
fn options_debug_redacts_password() {
    let opts = DbOptions::new("postgres://u:hunter2@127.0.0.1:1/x?password=hunter2");

    let dbg = format!("{opts:?}");

    assert!(!dbg.contains("hunter2"), "{dbg}");
    assert!(dbg.contains("127.0.0.1"), "{dbg}");
}

#[tokio::test]
async fn connect_failure_does_not_leak_password() {
    let mut opts = DbOptions::new("postgres://u:hunter2@127.0.0.1:1/x");
    opts.connect_timeout = std::time::Duration::from_secs(1);

    let err = connect(&opts).await.unwrap_err();

    for text in [format!("{err}"), format!("{err:?}"), format!("{opts:?}")] {
        assert!(!text.contains("hunter2"), "{text}");
    }
}

#[tokio::test]
async fn connect_reports_backend() {
    each_backend(async |db: &TestDb| {
        let expected = match db.backend {
            Backend::Sqlite => DbBackend::Sqlite,
            Backend::Postgres => DbBackend::Postgres,
        };
        assert_eq!(db.conn.get_database_backend(), expected);
        db.conn.execute_unprepared("SELECT 1").await.unwrap();
    })
    .await;
}
