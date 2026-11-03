//! `POST /devices/{id}/disconnect`: the connection id is queued and the device's next
//! heartbeat hands it back (the M3-05 acceptance). Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{DEVICE_IP, path, register, uuid_b64};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::{Heartbeat, HeartbeatOutcome, HeartbeatReply};
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

async fn beat(h: &Harness, rustdesk_id: &str, conns: &[i32]) -> HeartbeatReply {
    let beat = Heartbeat {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: uuid_b64(rustdesk_id),
        client: ClientAddr {
            addr: DEVICE_IP,
            is_proxy_fallback: false,
        },
        conns: conns.to_vec(),
        modified_at: 0,
    };
    match h.state.devices.heartbeat(beat).await.unwrap() {
        HeartbeatOutcome::Reply(reply) => reply,
        other => panic!("heartbeat refused: {other:?}"),
    }
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

        let reply = beat(&h, "100", &[7, 42]).await;
        assert_eq!(reply.disconnect, [42]);
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

        assert!(beat(&h, "100", &[]).await.disconnect.is_empty());
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
    })
    .await;
}
