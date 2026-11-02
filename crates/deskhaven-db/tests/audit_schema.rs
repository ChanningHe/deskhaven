//! Audit schema (M2-02): connection sessions, file transfers, alarms, nonces, login events and
//! the console audit log, on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::ops::{
    alarm_row, assert_checks, assert_indexes, connection_row, console_audit_row, file_transfer_row,
    is_fk_violation, is_unique_violation, login_event_row,
};
use common::{T0_MS, count, ts, user_row};
use deskhaven_db::entity::{
    alarm, audit_nonce, connection_session, console_audit, file_transfer, login_event, user,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

const TABLES: [&str; 6] = [
    "connection_sessions",
    "file_transfers",
    "alarms",
    "audit_nonces",
    "login_events",
    "console_audit",
];

#[tokio::test]
async fn tables_and_indexes_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let manager = SchemaManager::new(&db.conn);
        for table in TABLES {
            assert!(
                manager.has_table(table).await.unwrap(),
                "{}: missing {table}",
                db.backend
            );
        }
        assert_indexes(
            db,
            &[
                ("connection_sessions", "uq_connection_sessions_guid"),
                ("connection_sessions", "uq_connection_sessions_conn"),
                (
                    "connection_sessions",
                    "idx_connection_sessions_controller_user_id",
                ),
                ("connection_sessions", "idx_connection_sessions_created_at"),
                (
                    "connection_sessions",
                    "idx_connection_sessions_device_created",
                ),
                ("file_transfers", "idx_file_transfers_device_rustdesk_id"),
                ("file_transfers", "idx_file_transfers_created_at"),
                ("alarms", "idx_alarms_device_rustdesk_id"),
                ("alarms", "idx_alarms_created_at"),
                ("audit_nonces", "idx_audit_nonces_received_at"),
                ("login_events", "idx_login_events_user_created"),
                ("login_events", "idx_login_events_created_at"),
                ("console_audit", "idx_console_audit_created_at"),
            ],
        )
        .await;
    })
    .await;
}

// connection_sessions

#[tokio::test]
async fn a_close_message_alone_makes_a_valid_row() {
    each_migrated_backend(async |db: &TestDb| {
        // Messages of one connection arrive in any order; the first creates the row.
        let inserted = connection_row("111", 3, "s-1", "g-1")
            .insert(&db.conn)
            .await
            .unwrap();

        let found = connection_session::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();

        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_full_connection_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        let row = connection_session::ActiveModel {
            peer_rustdesk_id: Set(Some("222".to_owned())),
            peer_name: Set(Some("laptop".to_owned())),
            controller_user_id: Set(Some(alice.id)),
            conn_type: Set(Some(4)),
            primary_auth: Set(Some(2)),
            two_factor: Set(Some(1)),
            ip: Set(Some("203.0.113.9".to_owned())),
            started_at: Set(Some(ts(T0_MS))),
            authorized_at: Set(Some(ts(T0_MS + 1))),
            ended_at: Set(Some(ts(T0_MS + 2))),
            note: Set(Some("host note".to_owned())),
            controller_note: Set(Some("controller note".to_owned())),
            ..connection_row("111", 3, "s-1", "g-1")
        };

        let inserted = row.insert(&db.conn).await.unwrap();

        let found = connection_session::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();
        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_connection_is_unique_per_device_conn_and_session() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        connection_row("111", 3, "s-1", "g-1")
            .insert(conn)
            .await
            .unwrap();

        let dup = connection_row("111", 3, "s-1", "g-2").insert(conn).await;
        let other_device = connection_row("112", 3, "s-1", "g-3").insert(conn).await;
        let other_conn = connection_row("111", 4, "s-1", "g-4").insert(conn).await;
        let other_session = connection_row("111", 3, "s-2", "g-5").insert(conn).await;

        let b = db.backend;
        assert!(is_unique_violation(dup), "{b}: duplicate connection");
        assert!(other_device.is_ok(), "{b}: other device");
        assert!(other_conn.is_ok(), "{b}: other conn_id");
        assert!(other_session.is_ok(), "{b}: other session_id");
    })
    .await;
}

#[tokio::test]
async fn connection_guid_is_unique() {
    each_migrated_backend(async |db: &TestDb| {
        connection_row("111", 1, "s", "g")
            .insert(&db.conn)
            .await
            .unwrap();

        let dup = connection_row("112", 1, "s", "g").insert(&db.conn).await;

        assert!(is_unique_violation(dup), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn unknown_connection_types_are_stored() {
    each_migrated_backend(async |db: &TestDb| {
        // Newer clients may report types this server does not know; an audit insert must
        // never be rejected for that.
        for (conn_id, conn_type) in [(1, 5), (2, 99), (3, -1)] {
            let row = connection_session::ActiveModel {
                conn_type: Set(Some(conn_type)),
                ..connection_row("111", conn_id, "s", &format!("g-{conn_id}"))
            };

            let inserted = row.insert(&db.conn).await.unwrap();

            assert_eq!(inserted.conn_type, Some(conn_type), "{}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn controller_must_exist_and_is_cleared_when_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let dangling = connection_session::ActiveModel {
            controller_user_id: Set(Some(404)),
            ..connection_row("111", 1, "s", "g-1")
        };
        let linked = connection_session::ActiveModel {
            controller_user_id: Set(Some(alice.id)),
            ..connection_row("111", 2, "s", "g-2")
        };
        let res = dangling.insert(conn).await;
        linked.insert(conn).await.unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let b = db.backend;
        assert!(is_fk_violation(res), "{b}: dangling controller");
        let kept = connection_session::Entity::find().all(conn).await.unwrap();
        assert_eq!(kept.len(), 1, "{b}: the audit row survives");
        assert_eq!(kept[0].controller_user_id, None, "{b}");
    })
    .await;
}

// file_transfers

#[tokio::test]
async fn file_transfer_round_trips_and_checks_its_enums() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = file_transfer_row("111").insert(&db.conn).await.unwrap();
        let found = file_transfer::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();
        assert_eq!(found, Some(inserted), "{}", db.backend);

        assert_checks(
            db,
            &[
                (
                    "file_transfers",
                    "direction = 'sideways'",
                    "direction = 'receive'",
                ),
                ("file_transfers", "is_file = 2", "is_file = false"),
                ("file_transfers", "file_count = -1", "file_count = 0"),
            ],
        )
        .await;
    })
    .await;
}

// alarms

#[tokio::test]
async fn alarm_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = alarm_row("111").insert(&db.conn).await.unwrap();

        let found = alarm::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();

        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

// audit_nonces

#[tokio::test]
async fn a_nonce_is_recorded_once() {
    each_migrated_backend(async |db: &TestDb| {
        let nonce = |value: &str| audit_nonce::ActiveModel {
            nonce: Set(value.to_owned()),
            received_at: Set(ts(T0_MS)),
        };
        nonce("n-1").insert(&db.conn).await.unwrap();

        let dup = nonce("n-1").insert(&db.conn).await;
        let other = nonce("n-2").insert(&db.conn).await;

        assert!(is_unique_violation(dup), "{}", db.backend);
        assert!(other.is_ok(), "{}", db.backend);
        assert_eq!(count::<audit_nonce::Entity>(&db.conn).await, 2);
    })
    .await;
}

// login_events

#[tokio::test]
async fn login_event_round_trips_with_and_without_a_user() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();

        let known = login_event_row(Some(alice.id), "alice")
            .insert(&db.conn)
            .await
            .unwrap();
        let unknown = login_event::ActiveModel {
            rustdesk_id: Set(None),
            ip: Set(None),
            ..login_event_row(None, "mallory")
        };
        let unknown = unknown.insert(&db.conn).await.unwrap();

        let b = db.backend;
        let found = login_event::Entity::find_by_id(known.id)
            .one(&db.conn)
            .await
            .unwrap();
        assert_eq!(found, Some(known), "{b}");
        assert_eq!(unknown.user_id, None, "{b}");
    })
    .await;
}

#[tokio::test]
async fn login_event_enums_are_checked() {
    each_migrated_backend(async |db: &TestDb| {
        login_event_row(None, "mallory")
            .insert(&db.conn)
            .await
            .unwrap();

        assert_checks(
            db,
            &[
                (
                    "login_events",
                    "method = 'fingerprint'",
                    "method = 'recovery'",
                ),
                ("login_events", "client = 'toaster'", "client = 'api'"),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn login_event_user_must_exist_and_is_cleared_when_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let dangling = login_event_row(Some(404), "ghost").insert(conn).await;
        login_event_row(Some(alice.id), "alice")
            .insert(conn)
            .await
            .unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let b = db.backend;
        assert!(is_fk_violation(dangling), "{b}: dangling user");
        let kept = login_event::Entity::find().all(conn).await.unwrap();
        assert_eq!(kept.len(), 1, "{b}: the audit row survives");
        assert_eq!(kept[0].user_id, None, "{b}");
        assert_eq!(kept[0].username_attempted, "alice", "{b}");
    })
    .await;
}

// console_audit

#[tokio::test]
async fn console_audit_round_trips_for_each_actor_kind() {
    each_migrated_backend(async |db: &TestDb| {
        use console_audit::ActorKind::{ApiToken, Cli, System, User};
        for (kind, actor_id) in [
            (User, Some(1)),
            (ApiToken, Some(2)),
            (Cli, None),
            (System, None),
        ] {
            let row = console_audit::ActiveModel {
                actor_kind: Set(kind),
                ..console_audit_row(actor_id)
            };

            let inserted = row.insert(&db.conn).await.unwrap();

            let found = console_audit::Entity::find_by_id(inserted.id)
                .one(&db.conn)
                .await
                .unwrap();
            assert_eq!(found, Some(inserted), "{}: {kind:?}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn console_audit_actor_kind_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        console_audit_row(None).insert(&db.conn).await.unwrap();

        assert_checks(
            db,
            &[("console_audit", "actor_kind = 'bot'", "actor_kind = 'cli'")],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn console_audit_outlives_its_actor() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        console_audit_row(Some(alice.id))
            .insert(conn)
            .await
            .unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let kept = console_audit::Entity::find().all(conn).await.unwrap();
        assert_eq!(kept.len(), 1, "{}", db.backend);
        assert_eq!(kept[0].actor_id, Some(alice.id), "{}", db.backend);
    })
    .await;
}
