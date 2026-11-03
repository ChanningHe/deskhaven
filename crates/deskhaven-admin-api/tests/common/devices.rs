//! Device fixtures: rows registered like the signal server does, sysinfo reported like the
//! client, presence set like the heartbeat does.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_core::devices::{
    DeviceRepository, Presets, Registration, Sysinfo, SysinfoOutcome, SysinfoReport,
};
use deskhaven_db::time;

use super::Harness;

pub const DEVICE_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

/// The uuid every [`register`]ed device has, derived from its id.
pub fn uuid_of(rustdesk_id: &str) -> Vec<u8> {
    format!("uuid-of-{rustdesk_id}").into_bytes()
}

/// `uuid_of` as the client sends it.
pub fn uuid_b64(rustdesk_id: &str) -> String {
    STANDARD.encode(uuid_of(rustdesk_id))
}

/// Registers `rustdesk_id` (a RegisterPk); returns its `devices.id`.
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

/// The sysinfo upload of `rustdesk_id`.
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

/// A heartbeat of `rustdesk_id` reporting `conns`, straight into the presence table.
pub fn heartbeat(h: &Harness, rustdesk_id: &str, conns: &[i32]) {
    h.state
        .devices
        .presence()
        .heartbeat(rustdesk_id, DEVICE_IP, conns);
}

pub fn path(device_id: i64) -> String {
    format!("/admin/api/v1/devices/{device_id}")
}
