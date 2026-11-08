//! Alarm `info` is self-reported JSON: stored as a flat object of scalars, strings cleaned
//! and cut, keys bounded, nested values dropped (every client call site sends flat values,
//! `src/server/connection.rs:1439-1474`, `4303-4590`).

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::audit::{AlarmReport, MAX_INFO_KEYS, MAX_INFO_TEXT_CHARS, Record};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

use super::support::ctx;

#[tokio::test]
async fn alarm_info_is_stored_flat_cleaned_and_bounded() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let mut info = json!({
            "ip": "198.51.100.4", "name": "evil\u{202E}gpj.exe\u{0}", "id": 42, "a_ok": true,
            "nested": { "a": 1 }, "list": [1, 2], "none": null,
            "message": "x".repeat(MAX_INFO_TEXT_CHARS + 10),
            "bad\u{0}key": "v",
        });
        let object = info.as_object_mut().unwrap();
        for i in 0..MAX_INFO_KEYS {
            object.insert(format!("k{i:02}"), json!(i));
        }
        let record = Record::Alarm(AlarmReport {
            typ: 9,
            conn_id: 1,
            info: object.clone(),
        });

        // Act
        c.send("a", record).await;

        // Assert
        let b = db.backend;
        let alarms = c.alarms().await;
        let stored: Value = serde_json::from_str(&alarms[0].info).unwrap();
        let stored = stored.as_object().unwrap();
        assert_eq!(stored.len(), MAX_INFO_KEYS, "{b}");
        assert_eq!(stored["name"], json!("evilgpj.exe"), "{b}");
        assert_eq!(stored["ip"], json!("198.51.100.4"), "{b}");
        assert_eq!(
            (&stored["id"], &stored["a_ok"]),
            (&json!(42), &json!(true)),
            "{b}"
        );
        let message = stored["message"].as_str().unwrap();
        assert_eq!(message.chars().count(), MAX_INFO_TEXT_CHARS, "{b}");
        assert_eq!(
            stored.get("badkey"),
            Some(&json!("v")),
            "{b}: keys cleaned too"
        );
        let scalar = |v: &Value| !v.is_object() && !v.is_array() && !v.is_null();
        assert!(stored.values().all(scalar), "{b}");
    })
    .await;
}
