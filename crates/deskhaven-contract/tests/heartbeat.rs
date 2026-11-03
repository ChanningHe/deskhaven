//! `POST /api/heartbeat` flows (03 §2.1): sequences of heartbeats as the client sends them,
//! each reply decoded with the client mirror and applied the way `sync.rs:248-277` does.
//! Both database backends are covered by the core tests (`deskhaven-core/tests/devices`).
#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use deskhaven_client_api::{MAX_HEARTBEAT_BODY, TRY_LATER_MESSAGE};
use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::device_seeds::{DEVICE_ID, DEVICE_UUID_B64, GLOBAL_STRATEGY};
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::{ErrorBody, HeartbeatResponse};
use deskhaven_contract::requests::from;
use deskhaven_contract::seeds::Seeds;
use deskhaven_core::clock::ManualClock;
use deskhaven_core::devices::{HEARTBEAT_MIN_INTERVAL, HEARTBEAT_PER_IP_PER_MINUTE};
use deskhaven_core::strategies::StrategyUpdate;
use serde_json::{Value, json};

/// The client's tick with live connections.
const TICK: Duration = Duration::from_secs(3);

/// What the client keeps between heartbeats: `strategy_timestamp` and its options.
#[derive(Default)]
struct Client {
    modified_at: i64,
    options: HashMap<String, String>,
}

impl Client {
    fn request(&self, conns: &[i32]) -> Request {
        let mut body = json!({
            "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "ver": 1_004_001,
            "modified_at": self.modified_at,
        });
        if !conns.is_empty() {
            body["conns"] = json!(conns);
        }
        heartbeat_request(body)
    }

    /// Applies a reply as `sync.rs:261-277` and `handle_config_options` (no advanced
    /// defaults): an empty value removes the option.
    fn apply(&mut self, reply: &HeartbeatResponse) {
        if let Some(strategy) = &reply.strategy {
            for (key, value) in &strategy.config_options {
                match value.is_empty() {
                    true => drop(self.options.remove(key)),
                    false => drop(self.options.insert(key.clone(), value.clone())),
                }
            }
        }
        if let Some(modified_at) = reply.modified_at {
            self.modified_at = modified_at;
        }
    }
}

fn heartbeat_request(body: Value) -> Request {
    let mut request = Request::new("POST", "/api/heartbeat");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(body);
    request
}

struct Harness {
    app: TestApp,
    clock: Arc<ManualClock>,
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
        Seeds::builtin().apply(&names, &app.state).await.unwrap();
        Self { app, clock }
    }

    /// One tick later, the client's heartbeat; the reply is applied.
    async fn beat(&self, client: &mut Client) -> HeartbeatResponse {
        self.clock.advance(TICK);
        let reply = self.app.send(&client.request(&[])).await.unwrap();
        assert_eq!(reply.status, 200, "{}", text(&reply));
        let decoded: HeartbeatResponse = reply.json().unwrap();
        client.apply(&decoded);
        decoded
    }

    async fn set_global_options(&self, pairs: &[(&str, &str)]) {
        let strategies = &self.app.state.strategies;
        let strategy = strategies.get_by_name(GLOBAL_STRATEGY).await.unwrap();
        let options = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let update = StrategyUpdate {
            options: Some(options),
            ..StrategyUpdate::default()
        };
        strategies.update(strategy.id, update).await.unwrap();
    }
}

fn text(reply: &Reply) -> &str {
    std::str::from_utf8(&reply.body).unwrap_or("<binary>")
}

fn options(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[tokio::test]
async fn the_strategy_is_sent_until_the_client_holds_its_version() {
    // Arrange
    let h = Harness::start(&["registered-device", "device-sysinfo", "global-strategy"]).await;
    let mut client = Client::default();

    // Act
    let first = h.beat(&mut client).await;
    let second = h.beat(&mut client).await;

    // Assert
    assert!(first.strategy.is_some() && first.modified_at.is_some());
    assert_eq!(
        client.options,
        options(&[("enable-file-transfer", "N")]),
        "the empty option is deleted on the client"
    );
    assert!(second.strategy.is_none() && second.modified_at.is_none());
    assert!(second.sysinfo.is_none() && second.disconnect.is_none());
}

#[tokio::test]
async fn an_option_removed_from_the_strategy_is_sent_empty_once() {
    // Arrange
    let h = Harness::start(&["registered-device", "device-sysinfo", "global-strategy"]).await;
    h.set_global_options(&[("a", "1"), ("b", "2")]).await;
    let mut client = Client::default();
    h.beat(&mut client).await;
    h.beat(&mut client).await;

    // Act
    h.set_global_options(&[("a", "1")]).await;
    let removal = h.beat(&mut client).await;
    let confirmed = h.beat(&mut client).await;
    h.set_global_options(&[("a", "2")]).await;
    let next = h.beat(&mut client).await;

    // Assert
    let sent = |r: &HeartbeatResponse| r.strategy.as_ref().unwrap().config_options.clone();
    assert_eq!(sent(&removal), options(&[("a", "1"), ("b", "")]));
    assert!(confirmed.strategy.is_none());
    assert_eq!(sent(&next), options(&[("a", "2")]), "b is not sent again");
    assert_eq!(client.options, options(&[("a", "2")]));
}

#[tokio::test]
async fn a_heartbeat_within_the_minimum_interval_is_429() {
    // Arrange
    let h = Harness::start(&["registered-device"]).await;
    let client = Client::default();
    h.app.send(&client.request(&[])).await.unwrap();

    // Act
    h.clock
        .advance(HEARTBEAT_MIN_INTERVAL - Duration::from_millis(1));
    let early = h.app.send(&client.request(&[])).await.unwrap();
    h.clock.advance(Duration::from_millis(1));
    let on_time = h.app.send(&client.request(&[])).await.unwrap();

    // Assert
    assert_eq!(early.status, 429);
    assert_eq!(early.json::<ErrorBody>().unwrap().error, TRY_LATER_MESSAGE);
    assert_eq!(early.headers["retry-after"], "2");
    assert_eq!(on_time.status, 200);
}

/// Malformed requests and unknown ids (each costs a lookup) count against the client
/// network's quota; another network has its own.
#[tokio::test]
async fn malformed_and_unknown_heartbeats_over_the_per_ip_quota_are_429() {
    // Arrange
    let h = Harness::start(&[]).await;
    let garbage = heartbeat_request(json!({ "id": DEVICE_ID }));
    for _ in 0..HEARTBEAT_PER_IP_PER_MINUTE {
        let reply = h.app.send(&garbage).await.unwrap();
        assert_eq!(reply.status, 400);
    }

    // Act
    let limited = h.app.send(&garbage).await.unwrap();
    let unknown = h.app.send(&Client::default().request(&[])).await.unwrap();
    let elsewhere = h
        .app
        .send(&from(garbage, "198.51.100.9:4000"))
        .await
        .unwrap();

    // Assert
    assert_eq!(limited.status, 429);
    assert!(limited.headers.contains_key("retry-after"));
    assert_eq!(unknown.status, 429, "an unknown id needs a lookup");
    assert_eq!(elsewhere.status, 400, "another network has its own quota");
}

/// A known device (identity cached) costs no lookup: its heartbeats are not charged to the
/// network's quota, so a large NAT of verified devices is limited only per device.
#[tokio::test]
async fn heartbeats_of_known_devices_are_not_charged_to_the_network_quota() {
    // Arrange
    let h = Harness::start(&["registered-device"]).await;
    let mut client = Client::default();
    h.beat(&mut client).await;
    // Past the per-device interval before the quota is drained (time refills it).
    h.clock.advance(TICK);
    let garbage = heartbeat_request(json!({ "id": DEVICE_ID }));
    let mut drained = false;
    for _ in 0..=HEARTBEAT_PER_IP_PER_MINUTE {
        if h.app.send(&garbage).await.unwrap().status == 429 {
            drained = true;
            break;
        }
    }

    // Act
    let known = h.app.send(&client.request(&[])).await.unwrap();
    let malformed = h.app.send(&garbage).await.unwrap();

    // Assert
    assert!(drained, "the quota was used up");
    assert_eq!(known.status, 200, "{}", text(&known));
    assert_eq!(malformed.status, 429);
}

#[tokio::test]
async fn an_oversized_body_is_413() {
    // Arrange
    let h = Harness::start(&["registered-device"]).await;
    let conns: Vec<i32> = (0..i32::try_from(MAX_HEARTBEAT_BODY).unwrap()).collect();
    let request = Client::default().request(&conns);

    // Act
    let reply = h.app.send(&request).await.unwrap();

    // Assert
    assert_eq!(reply.status, 413);
}
