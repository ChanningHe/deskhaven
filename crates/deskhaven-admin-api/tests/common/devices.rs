// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{Method, StatusCode};
use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_core::devices::{
    DeviceRepository, Presets, Registration, Sysinfo, SysinfoOutcome, SysinfoReport,
};
use deskhaven_db::time;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{Harness, request};

pub const DEVICE_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

pub fn uuid_of(rustdesk_id: &str) -> Vec<u8> {
    format!("uuid-of-{rustdesk_id}").into_bytes()
}

pub fn uuid_b64(rustdesk_id: &str) -> String {
    STANDARD.encode(uuid_of(rustdesk_id))
}

pub async fn register(h: &Harness, rustdesk_id: &str) -> i64 {
    let repo = DeviceRepository::new(h.state.db.clone());
    let registration = Registration {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: uuid_of(rustdesk_id),
        pk: vec![1; 32],
        ip: DEVICE_IP,
        at: time::now(),
    };
    repo.upsert_registration(&registration).await.unwrap();
    let row = repo.find_by_rustdesk_id(rustdesk_id).await.unwrap();
    row.unwrap().id
}

pub async fn report(h: &Harness, rustdesk_id: &str, hostname: &str, os: &str) {
    let report = SysinfoReport {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: uuid_b64(rustdesk_id),
        sysinfo: Sysinfo {
            hostname: Some(hostname.to_owned()),
            username: Some("alice".to_owned()),
            os: Some(os.to_owned()),
            cpu: Some("Intel, 3.2GHz, 8/4 cores".to_owned()),
            memory: Some("15.5GB".to_owned()),
            version: Some("1.4.1".to_owned()),
        },
        presets: Presets::default(),
    };
    let outcome = h.state.devices.report_sysinfo(report).await.unwrap();
    assert_eq!(outcome, SysinfoOutcome::Stored);
}

pub fn heartbeat(h: &Harness, rustdesk_id: &str, conns: &[i32]) {
    h.state
        .devices
        .presence()
        .heartbeat(rustdesk_id, DEVICE_IP, conns);
}

pub fn path(device_id: i64) -> String {
    format!("/admin/api/v1/devices/{device_id}")
}

pub async fn heartbeat_http(
    h: &Harness,
    rustdesk_id: &str,
    conns: &[i32],
    modified_at: i64,
) -> (StatusCode, Value) {
    let router = apply_middleware(
        deskhaven_client_api::router().with_state(Arc::clone(&h.state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_client_api::render_stack_error),
    );
    let body = json!({
        "id": rustdesk_id,
        "uuid": uuid_b64(rustdesk_id),
        "conns": conns,
        "modified_at": modified_at,
    });
    let response = router
        .oneshot(request(Method::POST, "/api/heartbeat", &[], Some(body)))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
