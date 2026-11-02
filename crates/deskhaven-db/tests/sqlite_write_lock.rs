//! SQLite read-then-write transactions: deferred BEGIN fails on lock upgrade, `begin_write`
//! serializes writers instead.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use deskhaven_db::{DbOptions, begin_write, connect};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, TransactionTrait};

const POOL_SIZE: u32 = 4;
/// Long enough for the second writer to be parked in `BEGIN` while the first one works.
const OVERLAP: Duration = Duration::from_millis(200);

async fn counter_db(dir: &tempfile::TempDir) -> DatabaseConnection {
    let mut opts = DbOptions::new(format!("sqlite://{}", dir.path().join("c.db").display()));
    opts.max_connections = POOL_SIZE;
    let conn = connect(&opts).await.unwrap();
    conn.execute_unprepared(
        "CREATE TABLE counter (id INTEGER PRIMARY KEY, n INTEGER NOT NULL);
         INSERT INTO counter (id, n) VALUES (1, 0);",
    )
    .await
    .unwrap();
    conn
}

async fn read_n(db: &impl ConnectionTrait) -> Result<i64, DbErr> {
    let row = db
        .query_one_raw(sea_orm::Statement::from_string(
            db.get_database_backend(),
            "SELECT n FROM counter WHERE id = 1",
        ))
        .await?
        .ok_or_else(|| DbErr::RecordNotFound("counter".to_owned()))?;
    row.try_get_by_index(0)
}

async fn write_n(db: &impl ConnectionTrait, n: i64) -> Result<(), DbErr> {
    let sql = format!("UPDATE counter SET n = {n} WHERE id = 1");
    db.execute_unprepared(&sql).await.map(|_| ())
}

/// Documents the hazard: two deferred transactions both read; after the first commits its
/// write, the second cannot upgrade its stale snapshot and fails at once (no busy wait).
#[tokio::test]
async fn deferred_read_then_write_fails_with_busy() {
    let dir = tempfile::tempdir().unwrap();
    let conn = counter_db(&dir).await;
    let a = conn.begin().await.unwrap();
    let b = conn.begin().await.unwrap();
    read_n(&a).await.unwrap();
    read_n(&b).await.unwrap();

    write_n(&a, 1).await.unwrap();
    a.commit().await.unwrap();
    let err = write_n(&b, 1).await.unwrap_err();

    assert!(err.to_string().contains("locked"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn begin_write_serializes_read_then_write() {
    let dir = tempfile::tempdir().unwrap();
    let conn = counter_db(&dir).await;

    let a = begin_write(&conn).await.unwrap();
    let n = read_n(&a).await.unwrap();
    let conn_b = conn.clone();
    let b = tokio::spawn(async move {
        let b = begin_write(&conn_b).await?;
        let n = read_n(&b).await?;
        write_n(&b, n + 1).await?;
        b.commit().await
    });
    tokio::time::sleep(OVERLAP).await;
    write_n(&a, n + 1).await.unwrap();
    a.commit().await.unwrap();

    b.await.unwrap().unwrap();
    assert_eq!(read_n(&conn).await.unwrap(), 2, "no lost update");
}
