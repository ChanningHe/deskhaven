//! Opening web client sessions: the switch, authorization by device ownership (admins and
//! owners: any device), the signal server's failures as stable error codes, and the audit
//! row of every outcome. The signal server is a fake (`common::web_client`). Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::web_client::{
    ALICE_DEVICE, ANSWER, FakePunches, OFFER, OWNER_DEVICE, SESSIONS, UUID, enable, open,
    owned_device, setup,
};
use common::{Harness, PEER, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_core::web_client::PunchError;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

async fn web_rows(h: &Harness) -> Vec<console_audit::Model> {
    let rows = h.audit_rows().await;
    rows.into_iter()
        .filter(|r| r.action.starts_with("web_client."))
        .collect()
}

fn after(row: &console_audit::Model) -> Value {
    serde_json::from_str(row.after.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn a_session_needs_a_console_session_and_the_switch_on() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        FakePunches::install(&h);
        let owner = h.console("owner", Role::Owner).await;
        owned_device(&h, &owner, OWNER_DEVICE, owner.user_id).await;
        let body = json!({ "device_id": OWNER_DEVICE });

        // Act
        let anonymous = h
            .send(Method::POST, SESSIONS, None, Some("x"), Some(body))
            .await;
        let disabled = open(&h, &owner, OWNER_DEVICE, false).await;

        // Assert
        let b = db.backend;
        assert_eq!(anonymous.status, 401, "{b}");
        assert_eq!(disabled.status, 403, "{b}");
        assert_eq!(disabled.code(), "web_client_disabled", "{b}");
        let rows = web_rows(&h).await;
        assert_eq!(rows.len(), 1, "{b}: the refusal is audited");
        assert_eq!(after(&rows[0])["result"], "web_client_disabled", "{b}");
    })
    .await;
}

#[tokio::test]
async fn me_tells_every_role_whether_the_switch_is_on() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let user = h.console("uma", Role::User).await;

        // Act
        let off = h.get("/admin/api/v1/me", &user).await;
        enable(&h).await;
        let on = h.get("/admin/api/v1/me", &user).await;

        // Assert
        let b = db.backend;
        assert_eq!(off.body["web_client_enabled"], false, "{b}");
        assert_eq!(on.body["web_client_enabled"], true, "{b}");
    })
    .await;
}

#[tokio::test]
async fn without_a_signal_server_in_the_process_it_is_unavailable() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        enable(&h).await;
        let owner = h.console("owner", Role::Owner).await;
        owned_device(&h, &owner, OWNER_DEVICE, owner.user_id).await;

        // Act
        let reply = open(&h, &owner, OWNER_DEVICE, false).await;

        // Assert
        assert_eq!(reply.status, 503, "{}", db.backend);
        assert_eq!(reply.code(), "unavailable");
    })
    .await;
}

#[tokio::test]
async fn users_reach_their_own_devices_and_admins_every_device() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let admin = h.console("admin", Role::Admin).await;

        // Act
        let foreign = open(&h, &s.alice, OWNER_DEVICE, false).await;
        let unknown = open(&h, &s.alice, "999999999", false).await;
        let own = open(&h, &s.alice, ALICE_DEVICE, false).await;
        let by_admin = open(&h, &admin, ALICE_DEVICE, false).await;
        let by_owner = open(&h, &s.owner, ALICE_DEVICE, false).await;

        // Assert
        let b = db.backend;
        assert_eq!(foreign.status, 404, "{b}");
        assert_eq!(foreign.code(), "not_found", "{b}");
        assert_eq!(foreign.body, unknown.body, "{b}: existence does not leak");
        assert_eq!(unknown.status, 404, "{b}");
        assert_eq!(own.status, 201, "{b}: {:?}", own.body);
        assert_eq!(by_admin.status, 201, "{b}: {:?}", by_admin.body);
        assert_eq!(by_owner.status, 201, "{b}: {:?}", by_owner.body);
        assert_eq!(s.fake.punches(), 3, "{b}: refused requests punch nothing");
    })
    .await;
}

#[tokio::test]
async fn an_opened_session_carries_what_the_browser_needs() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;

        // Act
        let reply = open(&h, &s.alice, ALICE_DEVICE, true).await;

        // Assert
        let b = db.backend;
        assert_eq!(reply.status, 201, "{b}: {:?}", reply.body);
        let body = &reply.body;
        assert_eq!(body["peer_id"], ALICE_DEVICE);
        assert_eq!(body["signed_peer_pk"], "AQIDBA==");
        assert_eq!(body["server_pk"], "test-signal-key");
        assert_eq!(body["uuid"], UUID);
        assert_eq!(
            body["relay_url"], "/ws/relay",
            "no public_url: the console's origin"
        );
        assert_eq!(body["answer"], ANSWER);
        assert_eq!(body["my_id"], "alice");
        assert_eq!(body["my_name"], "alice");
        assert_eq!(body["ice_servers"].as_array().unwrap().len(), 4);
        assert_eq!(
            body["ice_servers"][0],
            json!({ "urls": ["stun:stun.cloudflare.com:3478"] })
        );
        assert_eq!(body["session_id"].as_str().unwrap().len(), 22);
        let routes = s.fake.take();
        assert_eq!(routes[0].request.device_id, ALICE_DEVICE);
        assert_eq!(routes[0].request.offer.as_deref(), Some(OFFER));
        assert_eq!(routes[0].request.client, PEER.ip(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn turn_credentials_go_with_turn_urls_only() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let urls = "stun:stun.example.com,turns:turn.example.com:5349?transport=tcp";
        h.set_setting("web_client.ice_servers", json!(urls)).await;
        h.set_setting("web_client.turn_username", json!("web"))
            .await;
        h.set_setting("web_client.turn_credential", json!("s3cret"))
            .await;

        // Act
        let reply = open(&h, &s.alice, ALICE_DEVICE, false).await;

        // Assert
        assert_eq!(reply.status, 201, "{}", db.backend);
        assert_eq!(reply.body.get("answer"), None, "no offer, no answer");
        assert_eq!(
            reply.body["ice_servers"],
            json!([
                { "urls": ["stun:stun.example.com"] },
                {
                    "urls": ["turns:turn.example.com:5349?transport=tcp"],
                    "username": "web",
                    "credential": "s3cret",
                },
            ])
        );
        let rows = h.audit_rows().await;
        assert!(rows.iter().all(|r| !format!("{r:?}").contains("s3cret")));
    })
    .await;
}

#[tokio::test]
async fn signal_server_failures_answer_stable_codes() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let foreign = PunchError::ForeignRelay {
            relay_server: "203.0.113.9:21117".to_owned(),
        };
        let cases = [
            (PunchError::NotFound, 404, "not_found"),
            (PunchError::Offline, 409, "device_offline"),
            (PunchError::NoAnswer, 504, "device_no_answer"),
            (foreign, 409, "foreign_relay"),
            (PunchError::NoRelay, 409, "no_relay"),
            (PunchError::Busy, 429, "rate_limited"),
            (PunchError::InvalidOffer, 400, "invalid_offer"),
            (PunchError::Stopped, 503, "unavailable"),
        ];

        for (err, status, code) in cases {
            // Act
            s.fake.fail_with(Some(err.clone()));
            let reply = open(&h, &s.owner, OWNER_DEVICE, true).await;

            // Assert
            let b = db.backend;
            assert_eq!(reply.status, status, "{b}: {err:?}");
            assert_eq!(reply.code(), code, "{b}: {err:?}");
            if code == "foreign_relay" {
                assert_eq!(
                    reply.body["error"]["details"]["relay_server"],
                    "203.0.113.9:21117"
                );
            }
            if code == "rate_limited" {
                assert!(reply.headers.contains_key("retry-after"), "{b}");
            }
        }
        let rows = web_rows(&h).await;
        let results: Vec<String> = rows
            .iter()
            .map(|r| after(r)["result"].to_string())
            .collect();
        assert_eq!(rows.len(), 8, "{}: every failure is audited", db.backend);
        assert!(results.iter().any(|r| r == "\"device_offline\""));
    })
    .await;
}

#[tokio::test]
async fn the_audit_row_names_user_device_ip_offer_and_outcome_but_no_sdp() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;

        // Act
        let reply = open(&h, &s.alice, ALICE_DEVICE, true).await;
        let id = reply.body["session_id"].as_str().unwrap().to_owned();
        let closed = h
            .act(Method::DELETE, &format!("{SESSIONS}/{id}"), &s.alice)
            .await;

        // Assert
        let b = db.backend;
        assert_eq!(closed.status, 204, "{b}");
        let rows = web_rows(&h).await;
        assert_eq!(rows.len(), 2, "{b}");
        let (opened, close) = (&rows[0], &rows[1]);
        assert_eq!(opened.action, "web_client.session.open");
        assert_eq!(opened.actor_id, Some(s.alice.user_id));
        assert_eq!(
            (opened.target_type.as_str(), opened.target_id.as_str()),
            ("device", ALICE_DEVICE)
        );
        assert_eq!(opened.ip.as_deref(), Some("203.0.113.9"));
        assert_eq!(
            after(opened),
            json!({ "offer": true, "result": "ok", "webrtc": true })
        );
        assert_eq!(close.action, "web_client.session.close");
        assert_eq!(close.target_id, ALICE_DEVICE);
        for row in &rows {
            let text = format!("{row:?}");
            for secret in ["webrtc://", "eyJ0eXBl", &id] {
                assert!(!text.contains(secret), "{b}: {secret} in {text}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn malformed_requests_are_refused_before_any_punch() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let h = harness(db, AdminRoutes::new()).await;
        let s = setup(&h).await;
        let long_offer = "x".repeat(4097);
        let bodies = [
            json!({ "device_id": "" }),
            json!({ "device_id": "../etc" }),
            json!({ "device_id": "a".repeat(65) }),
            json!({ "device_id": ALICE_DEVICE, "offer": long_offer }),
            json!({ "device_id": ALICE_DEVICE, "my_id": "spoofed" }),
        ];

        for body in bodies {
            // Act
            let reply = h
                .write(Method::POST, SESSIONS, &s.alice, body.clone())
                .await;

            // Assert
            assert_eq!(reply.status, 422, "{}: {body}", db.backend);
        }
        assert_eq!(s.fake.punches(), 0);
    })
    .await;
}
