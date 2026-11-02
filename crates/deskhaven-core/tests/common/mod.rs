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

pub mod lockout;

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

/// Log lines captured on the current thread (use with the current-thread test runtime).
pub struct CapturedLogs {
    buf: Arc<std::sync::Mutex<Vec<u8>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

impl CapturedLogs {
    pub fn take(&self) -> String {
        let bytes = std::mem::take(&mut *self.buf.lock().unwrap());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[derive(Clone)]
struct BufWriter(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for BufWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Captures every event at debug and above until the returned value is dropped.
pub fn capture_logs() -> CapturedLogs {
    let buf = Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = BufWriter(Arc::clone(&buf));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    CapturedLogs {
        buf,
        _guard: tracing::subscriber::set_default(subscriber),
    }
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
