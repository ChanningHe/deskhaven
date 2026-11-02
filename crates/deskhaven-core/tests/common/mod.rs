//! Shared setup for the login throttle tests.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::sync::Arc;

use deskhaven_config::settings::{Change, PinSet};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_db::testing::TestDb;
use serde_json::Value as Json;

/// A fixed instant with a non-zero millisecond part.
pub const T0_MS: i64 = 1_700_000_000_123;
pub const MINUTE: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn settings(db: &TestDb) -> SettingsService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (service, warnings) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{}: {warnings:?}", db.backend);
    service
}

pub fn clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::at(T0_MS))
}

/// Sets `key` to `value`, whatever its current version.
pub async fn set(settings: &SettingsService, key: &str, value: Json) {
    let version = settings.snapshot().view(key).unwrap().version;
    let change = Change {
        key: key.to_owned(),
        value,
        version,
    };
    settings
        .update(UpdateRequest {
            changes: vec![change],
            actor: None,
            reason: None,
        })
        .await
        .unwrap();
}
