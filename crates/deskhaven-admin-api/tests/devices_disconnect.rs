//! `POST /devices/{id}/disconnect`: the connection id is queued and the device's next
//! heartbeat, through the client API's `/api/heartbeat`, hands it back (the M3-05
//! acceptance); moving a device changes what its next heartbeat sends. Runs on SQLite and
//! PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::devices::{heartbeat_http, path, register};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::devices::MAX_QUEUED_DEVICES;
use deskhaven_core::strategies::{NewStrategy, Options, Target};
use deskhaven_core::users::Role;
use deskhaven_db::entity::user;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

/// The heartbeat of one device is limited to one per `HEARTBEAT_MIN_INTERVAL` (2 s).
const BEAT_GAP: Duration = Duration::from_millis(2_100);

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

fn disconnect_path(device_id: i64) -> String {
    format!("{}/disconnect", path(device_id))
}

#[tokio::test]
async fn the_next_heartbeat_returns_the_requested_connection() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;

        let queued = h
            .write(
                Method::POST,
                &disconnect_path(id),
                &admin,
                json!({ "conn_id": 42 }),
            )
            .await;
        assert_eq!(queued.status, 202, "{}", queued.body);
        assert!(queued.body["ttl_seconds"].as_u64().unwrap() > 0);

        let (status, reply) = heartbeat_http(&h, "100", &[7, 42], 0).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reply["disconnect"], json!([42]), "{reply}");
        assert!(
            !h.state.devices.disconnects().is_pending("100"),
            "handed out once"
        );
    })
    .await;
}

#[tokio::test]
async fn a_connection_the_device_does_not_report_stays_queued() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let body = json!({ "conn_id": 42 });
        h.write(Method::POST, &disconnect_path(id), &admin, body)
            .await;

        let (status, reply) = heartbeat_http(&h, "100", &[], 0).await;
        assert_eq!(status, StatusCode::OK);
        assert!(reply.get("disconnect").is_none(), "{reply}");
        assert!(h.state.devices.disconnects().is_pending("100"));
    })
    .await;
}

#[tokio::test]
async fn disconnect_is_audited_and_validated() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let missing = h
            .write(
                Method::POST,
                &disconnect_path(id + 1000),
                &admin,
                json!({ "conn_id": 1 }),
            )
            .await;
        assert_eq!(
            (missing.status.as_u16(), missing.code()),
            (404, "not_found")
        );
        for bad in [
            json!({}),
            json!({ "conn_id": "x" }),
            json!({ "conn_id": 5_000_000_000_i64 }),
        ] {
            let reply = h
                .write(Method::POST, &disconnect_path(id), &admin, bad)
                .await;
            assert_eq!(reply.status, 422, "{}", reply.body);
        }
        assert!(h.audit_rows().await.is_empty());
        assert!(!h.state.devices.disconnects().is_pending("100"));

        h.write(
            Method::POST,
            &disconnect_path(id),
            &admin,
            json!({ "conn_id": 9 }),
        )
        .await;
        let rows = h.audit_rows().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "device.disconnect");
        assert_eq!(rows[0].target_id, id.to_string());
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        assert_eq!(after["conn_id"], 9);
        assert_eq!(after["rustdesk_id"], "100");
    })
    .await;
}

#[tokio::test]
async fn a_full_queue_answers_503_with_the_attempt_audited_and_nothing_queued() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        for n in 0..MAX_QUEUED_DEVICES {
            let queue = h.state.devices.disconnects();
            queue.disconnect(&format!("filler-{n}"), &[1]).unwrap();
        }

        let refused = h
            .write(
                Method::POST,
                &disconnect_path(id),
                &admin,
                json!({ "conn_id": 42 }),
            )
            .await;

        assert_eq!(
            (refused.status.as_u16(), refused.code()),
            (503, "unavailable")
        );
        assert!(!h.state.devices.disconnects().is_pending("100"));
        let rows = h.audit_rows().await;
        assert_eq!(rows.len(), 1, "committed before the enqueue");
        assert_eq!(rows[0].action, "device.disconnect");
    })
    .await;
}

/// Changes `user_id`'s role behind the services' back: the session cache keeps the old role,
/// as for a demotion racing an in-flight request.
async fn demote_quietly(h: &Harness, user_id: i64, role: Role) {
    user::Entity::update_many()
        .col_expr(user::Column::Role, Expr::value(role))
        .filter(user::Column::Id.eq(user_id))
        .exec(&h.state.db)
        .await
        .unwrap();
}

#[tokio::test]
async fn an_actor_demoted_since_the_session_started_queues_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        demote_quietly(&h, admin.user_id, Role::User).await;

        let body = json!({ "conn_id": 42 });
        let refused = h
            .write(Method::POST, &disconnect_path(id), &admin, body)
            .await;

        assert_eq!(
            (refused.status.as_u16(), refused.code()),
            (403, "forbidden")
        );
        assert!(!h.state.devices.disconnects().is_pending("100"));
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

fn options(pairs: &[(&str, &str)]) -> Options {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[tokio::test]
async fn moving_a_device_makes_its_heartbeat_delete_the_old_groups_keys() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let (first, second) = (
            h.state.devices.create_group("A", "").await.unwrap(),
            h.state.devices.create_group("B", "").await.unwrap(),
        );
        for (group, key) in [(first, "key-a"), (second, "key-b")] {
            let new = NewStrategy::new(key, options(&[(key, "v")]));
            let strategy = h.state.strategies.create(new).await.unwrap();
            let target = Target::DeviceGroup(group);
            h.state
                .strategies
                .assign(target, strategy.id)
                .await
                .unwrap();
        }
        let join = json!({ "device_group_id": first });
        h.write(Method::PATCH, &path(id), &admin, join).await;
        let (_, joined) = heartbeat_http(&h, "100", &[], 0).await;
        assert_eq!(
            joined["strategy"]["config_options"],
            json!({ "key-a": "v" })
        );

        let moved = json!({ "device_group_id": second });
        h.write(Method::PATCH, &path(id), &admin, moved).await;
        tokio::time::sleep(BEAT_GAP).await;
        let (status, left) =
            heartbeat_http(&h, "100", &[], joined["modified_at"].as_i64().unwrap()).await;

        assert_eq!(status, StatusCode::OK);
        let expected = json!({ "key-a": "", "key-b": "v" });
        assert_eq!(left["strategy"]["config_options"], expected, "{left}");
    })
    .await;
}
