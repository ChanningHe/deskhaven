//! The three messages of one connection (new, authorized, close) merge into one
//! `connection_sessions` row whatever order they arrive in. The real client sends `new`
//! before the login request, with session id 0; the others carry the controller's session id.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::{Ingest, PAIRING_WINDOW, Record};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{PEER, SESSION, T0_MS, authorized, closed, ctx, new_conn};

const ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

fn messages(conn_id: i64, new_session: Option<u64>) -> [Record; 3] {
    let new = match new_session {
        None => new_conn(conn_id),
        Some(session) => {
            let Record::Conn(mut report) = new_conn(conn_id) else {
                unreachable!()
            };
            report.session_id = super::support::session(session);
            Record::Conn(report)
        }
    };
    [new, authorized(conn_id, SESSION), closed(conn_id, SESSION)]
}

async fn merges_in_every_order(db: &TestDb, new_session: Option<u64>) {
    // Arrange
    let c = ctx(db).await;

    // Act
    for (i, order) in ORDERS.iter().enumerate() {
        let conn_id = 1000 + i as i64;
        let mut parts = messages(conn_id, new_session).map(Some);
        for (step, &index) in order.iter().enumerate() {
            let record = parts[index].take().unwrap();
            let outcome = c.send(&format!("n-{i}-{step}"), record).await;
            assert_eq!(outcome, Ingest::Stored, "{}: order {order:?}", db.backend);
        }
    }

    // Assert
    let rows = c.sessions().await;
    assert_eq!(
        rows.len(),
        ORDERS.len(),
        "{}: one row per connection",
        db.backend
    );
    for (i, row) in rows.iter().enumerate() {
        let what = format!("{}: order {:?}", db.backend, ORDERS[i]);
        assert_eq!(row.conn_id, 1000 + i as i64, "{what}");
        assert_eq!(row.session_id, SESSION.to_string(), "{what}");
        assert_eq!(row.device_rustdesk_id, super::support::ID, "{what}");
        assert_eq!(row.ip.as_deref(), Some("198.51.100.4"), "{what}");
        assert_eq!(row.peer_rustdesk_id.as_deref(), Some(PEER), "{what}");
        assert_eq!(row.peer_name.as_deref(), Some("alice@laptop"), "{what}");
        let auth = (row.conn_type, row.primary_auth, row.two_factor);
        assert_eq!(auth, (Some(0), Some(2), Some(1)), "{what}");
        let times = [row.started_at, row.authorized_at, row.ended_at];
        assert!(times.iter().all(Option::is_some), "{what}: {times:?}");
        assert_eq!(row.created_at.timestamp_millis(), T0_MS, "{what}");
        assert_eq!(row.guid.len(), 36, "{what}");
    }
}

#[tokio::test]
async fn the_three_messages_merge_in_every_order() {
    each_migrated_backend(async |db: &TestDb| merges_in_every_order(db, None).await).await;
}

/// The contract's description of the messages: every one carries the same session id.
#[tokio::test]
async fn messages_that_all_carry_the_session_id_merge_too() {
    each_migrated_backend(async |db: &TestDb| merges_in_every_order(db, Some(SESSION)).await).await;
}

#[tokio::test]
async fn a_number_and_a_string_session_id_name_the_same_connection() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let Record::Conn(mut close) = closed(7, 0) else {
            unreachable!()
        };
        close.session_id =
            deskhaven_core::audit::SessionId::from_json(&serde_json::json!(SESSION.to_string()))
                .unwrap();

        // Act
        c.send("a", authorized(7, SESSION)).await;
        c.send("b", Record::Conn(close)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 1, "{}", db.backend);
        assert!(rows[0].ended_at.is_some(), "{}", db.backend);
    })
    .await;
}

/// A connection id is a per-process counter of the client: after a restart an old id can
/// name a new connection, so an opening is only paired within [`PAIRING_WINDOW`].
#[tokio::test]
async fn an_opening_is_not_paired_outside_the_window() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;

        // Act
        c.clock.advance(PAIRING_WINDOW + Duration::from_millis(1));
        c.send("b", authorized(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 2, "{}", db.backend);
        assert_eq!(rows[0].session_id, "0", "{}", db.backend);
        assert!(rows[0].authorized_at.is_none(), "{}", db.backend);
        assert!(rows[1].started_at.is_none(), "{}", db.backend);
    })
    .await;
}

/// A connection closed before its login request (new and close both with session 0) is
/// finished: a later authorization of a reused connection id starts a row of its own.
#[tokio::test]
async fn a_connection_closed_before_login_is_not_adopted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", new_conn(7)).await;
        c.send("b", closed(7, 0)).await;

        // Act
        c.send("c", authorized(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 2, "{}", db.backend);
        assert!(rows[0].ended_at.is_some() && rows[0].started_at.is_some());
        assert_eq!(rows[1].session_id, SESSION.to_string(), "{}", db.backend);
    })
    .await;
}

/// Audit tables are append-only for the API (06 §7): a second message of a kind already
/// merged fills nothing in again.
#[tokio::test]
async fn the_first_message_of_a_kind_wins() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.send("a", closed(7, SESSION)).await;
        let first = c.sessions().await[0].ended_at;

        // Act
        c.clock.advance(Duration::from_secs(5));
        c.send("b", closed(7, SESSION)).await;

        // Assert
        let rows = c.sessions().await;
        assert_eq!(rows.len(), 1, "{}", db.backend);
        assert_eq!(rows[0].ended_at, first, "{}", db.backend);
    })
    .await;
}
