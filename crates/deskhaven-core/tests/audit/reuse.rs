//! Connection ids are a per-process counter of the client: after a restart an old id names a
//! new connection. A finished or stale unbound row (session id 0) must not swallow the new
//! connection's records. And records of one connection racing each other still merge.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::{Ingest, PAIRING_WINDOW};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{SESSION, authorized, closed, ctx, new_conn};

/// Session id prefix of an unbound row whose key was freed for a later connection.
const RETIRED: &str = "0#";

#[tokio::test]
async fn a_connection_closed_before_login_frees_its_conn_id() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;
        c.send("b", closed(7, 0)).await;

        // Act
        let reopened = c.send("c", new_conn(7)).await;
        let reclosed = c.send("d", closed(7, 0)).await;

        // Assert
        let b = db.backend;
        assert_eq!(
            (reopened, reclosed),
            (Ingest::Stored, Ingest::Stored),
            "{b}"
        );
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 2, "{b}");
        assert!(
            rows[0].session_id.starts_with(RETIRED),
            "{b}: {}",
            rows[0].session_id
        );
        assert_eq!(rows[1].session_id, "0", "{b}");
        for row in &rows {
            assert!(
                row.started_at.is_some() && row.ended_at.is_some(),
                "{b}: {row:?}"
            );
        }
    })
    .await;
}

/// A connection that never closed (the client crashed before login) leaves an open unbound
/// row; past the pairing window it is not the same connection.
#[tokio::test]
async fn a_stale_unbound_row_does_not_take_a_new_connection() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;
        let stale = c.sessions().await[0].clone();
        c.clock.advance(PAIRING_WINDOW + Duration::from_secs(1));

        // Act
        let opened = c.send("b", new_conn(7)).await;
        c.send("c", authorized(7, SESSION)).await;

        // Assert
        let b = db.backend;
        assert_eq!(opened, Ingest::Stored, "{b}");
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 2, "{b}");
        assert!(rows[0].session_id.starts_with(RETIRED), "{b}");
        assert_eq!(
            rows[0].started_at, stale.started_at,
            "{b}: the old row is untouched"
        );
        assert!(rows[0].authorized_at.is_none(), "{b}");
        assert_eq!(rows[1].session_id, SESSION.to_string(), "{b}");
        let ms = |t: Option<deskhaven_db::time::TimestampMs>| t.map(|t| t.timestamp_millis());
        assert!(ms(rows[1].started_at) > ms(stale.started_at), "{b}");
        assert!(rows[1].authorized_at.is_some(), "{b}");
    })
    .await;
}

/// The opening and the authorization of one connection arriving at once: on PostgreSQL (READ
/// COMMITTED) both would see no row and insert two rows under different keys.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_records_of_one_connection_merge() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        const CONNS: i64 = 20;

        // Act
        let mut tasks = Vec::new();
        for conn_id in 0..CONNS {
            for (kind, record) in [
                ("n", new_conn(conn_id)),
                ("a", authorized(conn_id, SESSION)),
            ] {
                let audit = c.audit.clone();
                let envelope = super::support::envelope(&format!("{kind}-{conn_id}"));
                tasks.push(tokio::spawn(
                    async move { audit.ingest(envelope, record).await },
                ));
            }
        }
        for task in tasks {
            assert_eq!(
                task.await.unwrap().unwrap(),
                Ingest::Stored,
                "{}",
                db.backend
            );
        }

        // Assert
        let rows = c.sessions().await;
        assert_eq!(rows.len(), CONNS as usize, "{}", db.backend);
        assert!(
            rows.iter()
                .all(|r| r.started_at.is_some() && r.authorized_at.is_some()),
            "{}",
            db.backend
        );
    })
    .await;
}
