//! Web client sessions at the edges: a request dropped while the device is being punched,
//! and disabled devices. The signal server is a fake (`common::web_client`). Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::Method;
use common::web_client::{ALICE_DEVICE, SESSIONS, open, opened, setup};
use common::{auth_headers, harness, request};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::web_client::MAX_SESSIONS_PER_USER;
use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

const STEP: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);

#[tokio::test]
async fn a_request_dropped_mid_punch_is_audited_and_its_session_closed() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let release = s.fake.park_next();
        let body = json!({ "device_id": ALICE_DEVICE });
        let call = h.call(request(
            Method::POST,
            SESSIONS,
            &auth_headers(&s.alice, &[]),
            Some(body),
        ));

        // Act
        tokio::select! {
            _ = call => panic!("the punch is parked"),
            () = async { while s.fake.punches() == 0 { sleep(POLL).await } } => {}
        }
        release.send(()).unwrap();
        let deadline = Instant::now() + STEP;
        let row = loop {
            let rows = h.audit_rows().await;
            if let Some(row) = rows
                .into_iter()
                .find(|r| r.action == "web_client.session.open")
            {
                break row;
            }
            assert!(
                Instant::now() < deadline,
                "the dropped request was never audited"
            );
            sleep(POLL).await;
        };

        // Assert
        let b = db.backend;
        let after: Value = serde_json::from_str(row.after.as_deref().unwrap()).unwrap();
        assert_eq!(after["result"], "cancelled", "{b}");
        assert_eq!(row.actor_id, Some(s.alice.user_id), "{b}");
        for _ in 0..MAX_SESSIONS_PER_USER {
            opened(&h, &s.alice, ALICE_DEVICE).await;
        }
    })
    .await;
}

#[tokio::test]
async fn a_disabled_device_is_not_found_for_its_owner_and_for_admins() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let repo = DeviceRepository::new(h.state.db.clone());
        let row = repo
            .find_by_rustdesk_id(ALICE_DEVICE)
            .await
            .unwrap()
            .unwrap();
        let disable = device::ActiveModel {
            id: Set(row.id),
            status: Set(DeviceStatus::Disabled),
            ..Default::default()
        };
        disable.update(&h.state.db).await.unwrap();

        // Act
        let unknown = open(&h, &s.alice, "999999999", false).await;
        let by_owner = open(&h, &s.alice, ALICE_DEVICE, false).await;
        let by_admin = open(&h, &s.owner, ALICE_DEVICE, false).await;

        // Assert
        let b = db.backend;
        assert_eq!(unknown.status, 404, "{b}");
        for reply in [&by_owner, &by_admin] {
            assert_eq!(reply.status, 404, "{b}");
            assert_eq!(reply.body, unknown.body, "{b}");
        }
        assert_eq!(s.fake.punches(), 0, "{b}");
    })
    .await;
}
