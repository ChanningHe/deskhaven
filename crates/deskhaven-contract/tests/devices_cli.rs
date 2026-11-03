//! `--deploy` / `--assign` flows (03 §2.8, §2.9) through the real client API: what the
//! fixtures' replies leave behind, and the heartbeat that follows an assignment.
//! Both database backends are covered by the core tests (`deskhaven-core/tests/devices`).
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::device_seeds::{
    DEVICE_ID, DEVICE_UUID_B64, PRESET_DEVICE_GROUP, PRESET_STRATEGY, PRESET_USER,
};
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::{DeployResponse, HeartbeatResponse};
use deskhaven_contract::seeds::{SeedOutputs, Seeds};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::strategies::Target;
use deskhaven_db::entity::console_audit::{self, ActorKind};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

const PK_B64: &str = "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQU=";

struct Harness {
    app: TestApp,
    clock: Arc<ManualClock>,
    token: String,
}

impl Harness {
    async fn start(seeds: &[&str]) -> Self {
        let clock = Arc::new(ManualClock::at(1_730_000_000_000));
        let options = TestOptions {
            clock: Some(clock.clone()),
            ..TestOptions::default()
        };
        let app = TestApp::start_with_options(options).await.unwrap();
        let names: Vec<String> = seeds.iter().map(|s| (*s).to_owned()).collect();
        let outputs: SeedOutputs = Seeds::builtin().apply(&names, &app.state).await.unwrap();
        let token = outputs.get("api_token").cloned().unwrap_or_default();
        Self { app, clock, token }
    }

    fn post(&self, path: &str, body: Value, bearer: bool) -> Request {
        let mut request = Request::new("POST", path);
        request
            .headers
            .insert("content-type".to_owned(), "application/json".to_owned());
        if bearer {
            let value = format!("Bearer {}", self.token);
            request.headers.insert("authorization".to_owned(), value);
        }
        request.body = Body::Json(body);
        request
    }

    async fn assign(&self, extra: Value) -> Reply {
        let mut body = json!({ "id": DEVICE_ID, "uuid": DEVICE_UUID_B64 });
        if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
            body.extend(extra);
        }
        let request = self.post("/api/devices/cli", body, true);
        self.app.send(&request).await.unwrap()
    }

    /// A heartbeat 3 s after the last, holding strategy version `modified_at`.
    async fn beat(&self, modified_at: i64) -> HeartbeatResponse {
        self.clock.advance(Duration::from_secs(3));
        let body = json!({
            "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "ver": 1_004_001, "modified_at": modified_at,
        });
        let reply = self
            .app
            .send(&self.post("/api/heartbeat", body, false))
            .await
            .unwrap();
        assert_eq!(reply.status, 200, "{}", text(&reply));
        reply.json().unwrap()
    }
}

fn text(reply: &Reply) -> &str {
    std::str::from_utf8(&reply.body).unwrap_or("<binary>")
}

#[tokio::test]
async fn after_an_assignment_the_next_heartbeat_delivers_the_strategy() {
    // Arrange
    let h = Harness::start(&[
        "registered-device",
        "device-sysinfo",
        "device-preset-targets",
        "api-token",
    ])
    .await;
    let before = h.beat(0).await;
    let held = before.modified_at.unwrap_or(0);
    assert!(before.strategy.is_none(), "no strategy applies yet");

    // Act
    let reply = h.assign(json!({ "strategy_name": PRESET_STRATEGY })).await;
    let after = h.beat(held).await;

    // Assert
    assert_eq!((reply.status, text(&reply)), (200, ""));
    let strategy = after.strategy.unwrap();
    assert_eq!(
        strategy
            .config_options
            .get("enable-file-transfer")
            .map(String::as_str),
        Some("N")
    );
    assert!(after.modified_at.is_some_and(|v| v != held));
}

#[tokio::test]
async fn an_assignment_places_the_device_and_is_audited_as_the_token() {
    // Arrange
    let h = Harness::start(&["registered-device", "device-preset-targets", "api-token"]).await;

    // Act
    let reply = h
        .assign(json!({
            "user_name": PRESET_USER, "device_group_name": PRESET_DEVICE_GROUP,
            "strategy_name": PRESET_STRATEGY, "note": "front desk", "device_name": "reception-01",
            "address_book_name": "Team", "address_book_password": "s3cret-pass",
        }))
        .await;

    // Assert
    assert_eq!((reply.status, text(&reply)), (200, ""));
    let state = &h.app.state;
    let row = DeviceRepository::new(state.db.clone())
        .find_by_rustdesk_id(DEVICE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(row.owner_user_id.is_some() && row.device_group_id.is_some());
    assert_eq!(row.note.as_deref(), Some("front desk"));
    assert_eq!(row.display_name.as_deref(), Some("reception-01"));
    let assigned = state.strategies.assignment(Target::Device(row.id)).await;
    assert!(assigned.unwrap().is_some());
    let audit = console_audit::Entity::find().all(&state.db).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].actor_kind, ActorKind::ApiToken);
    assert_eq!(audit[0].action, "device.assign");
    assert_eq!(audit[0].target_id, DEVICE_ID);
    let after = audit[0].after.clone().unwrap_or_default();
    assert!(!after.contains("s3cret-pass"), "{after}");
}

#[tokio::test]
async fn a_deploy_marks_the_device_deployed_and_is_audited() {
    // Arrange
    let h = Harness::start(&["registered-device", "deploy-required", "api-token"]).await;
    let body = json!({ "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "pk": PK_B64 });

    // Act
    let reply = h
        .app
        .send(&h.post("/api/devices/deploy", body, true))
        .await
        .unwrap();

    // Assert
    let decoded: DeployResponse = reply.json().unwrap();
    assert_eq!(decoded.result, "OK");
    let state = &h.app.state;
    let row = DeviceRepository::new(state.db.clone())
        .find_by_rustdesk_id(DEVICE_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(row.deployed_at.is_some());
    let audit = console_audit::Entity::find().all(&state.db).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(
        (audit[0].actor_kind, audit[0].action.as_str()),
        (ActorKind::ApiToken, "device.deploy")
    );
}

#[tokio::test]
async fn a_revoked_token_is_refused() {
    // Arrange
    let h = Harness::start(&["registered-device", "deploy-required", "api-token"]).await;
    let tokens = &h.app.state.api_tokens;
    let id = tokens.list().await.unwrap()[0].id;
    tokens.revoke(id).await.unwrap();

    // Act
    let reply = h.assign(json!({ "note": "front desk" })).await;

    // Assert
    assert_eq!(reply.status, 401, "{}", text(&reply));
}

#[tokio::test]
async fn a_malformed_body_is_invalid_input_for_deploy_and_a_400_for_assign() {
    let h = Harness::start(&["registered-device", "deploy-required", "api-token"]).await;
    let mut deploy = h.post("/api/devices/deploy", json!(null), true);
    deploy.body = Body::Text("not json".to_owned());
    let mut assign = h.post("/api/devices/cli", json!(null), true);
    assign.body = Body::Text("not json".to_owned());

    let deployed = h.app.send(&deploy).await.unwrap();
    let assigned = h.app.send(&assign).await.unwrap();

    let decoded: DeployResponse = deployed.json().unwrap();
    assert_eq!(
        (deployed.status, decoded.result.as_str()),
        (200, "INVALID_INPUT")
    );
    assert_eq!(assigned.status, 400, "{}", text(&assigned));
}
