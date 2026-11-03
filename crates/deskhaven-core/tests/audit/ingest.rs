//! Nonce de-duplication, device identity, and the file / alarm records.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};

use deskhaven_core::audit::{
    AlarmReport, Direction, Envelope, FileInfo, FileReport, Ingest, Record,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

use super::support::{ID, PEER, SESSION, ctx, envelope, new_conn, uuid_b64};

fn file(path: &str) -> Record {
    Record::File(FileReport {
        peer_id: PEER.to_owned(),
        conn_id: 12,
        direction: Direction::Send,
        path: path.to_owned(),
        is_file: false,
        info: FileInfo {
            ip: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4))),
            file_count: 3,
            files: vec![("a.txt".to_owned(), 10), ("b\u{202E}txt.exe".to_owned(), 5)],
        },
    })
}

fn alarm() -> Record {
    let info = json!({ "ip": "198.51.100.4" });
    Record::Alarm(AlarmReport {
        typ: 1,
        conn_id: 12,
        info: info.as_object().unwrap().clone(),
    })
}

#[tokio::test]
async fn a_repeated_nonce_is_a_duplicate_and_stores_nothing_again() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let b = db.backend;

        // Act
        let first = [
            c.send("conn", new_conn(7)).await,
            c.send("file", file("/home")).await,
            c.send("alarm", alarm()).await,
        ];
        let retried = [
            c.send("conn", super::support::closed(7, SESSION)).await,
            c.send("file", file("/home")).await,
            c.send("alarm", alarm()).await,
        ];

        // Assert
        assert_eq!(first, [Ingest::Stored; 3], "{b}");
        assert_eq!(retried, [Ingest::Duplicate; 3], "{b}");
        let sessions = c.sessions().await;
        assert_eq!(sessions.len(), 1, "{b}");
        assert!(
            sessions[0].ended_at.is_none(),
            "{b}: the duplicate merged nothing"
        );
        assert_eq!(
            (c.files().await.len(), c.alarms().await.len()),
            (1, 1),
            "{b}"
        );
        assert_eq!(c.nonces().await, 3, "{b}");
    })
    .await;
}

#[tokio::test]
async fn unknown_devices_and_wrong_uuids_store_nothing_and_claim_no_nonce() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let unknown = Envelope {
            rustdesk_id: "555555555".to_owned(),
            uuid: uuid_b64("555555555"),
            nonce: Some("n".to_owned()),
        };
        let wrong_uuid = Envelope {
            uuid: uuid_b64("555555555"),
            ..envelope("n")
        };

        // Act
        let outcomes = [
            c.audit.ingest(unknown.clone(), new_conn(7)).await.unwrap(),
            c.audit.ingest(unknown.clone(), file("/")).await.unwrap(),
            c.audit.ingest(unknown, alarm()).await.unwrap(),
            c.audit.ingest(wrong_uuid, new_conn(7)).await.unwrap(),
        ];

        // Assert
        let b = db.backend;
        assert_eq!(outcomes, [Ingest::NotFound; 4], "{b}");
        assert!(
            c.sessions().await.is_empty() && c.files().await.is_empty(),
            "{b}"
        );
        assert!(c.alarms().await.is_empty(), "{b}");
        assert_eq!(c.nonces().await, 0, "{b}");
        assert_eq!(c.send("n", new_conn(7)).await, Ingest::Stored, "{b}");
    })
    .await;
}

/// Clients from before the nonce never retry, so their records are stored as they come.
#[tokio::test]
async fn records_without_a_nonce_are_stored_without_de_duplication() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let bare = Envelope {
            nonce: None,
            ..envelope("unused")
        };

        // Act
        let first = c.audit.ingest(bare.clone(), alarm()).await.unwrap();
        let second = c.audit.ingest(bare, alarm()).await.unwrap();

        // Assert
        assert_eq!(
            (first, second),
            (Ingest::Stored, Ingest::Stored),
            "{}",
            db.backend
        );
        assert_eq!(c.alarms().await.len(), 2, "{}", db.backend);
        assert_eq!(c.nonces().await, 0, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn file_and_alarm_records_keep_what_the_device_reported() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;

        // Act
        c.send("f", file("/home/alice\u{0}/docs")).await;
        c.send("a", alarm()).await;

        // Assert
        let b = db.backend;
        let files = c.files().await;
        let f = &files[0];
        assert_eq!(
            (f.device_rustdesk_id.as_str(), f.peer_rustdesk_id.as_str()),
            (ID, PEER)
        );
        assert_eq!(
            (f.conn_id, f.direction, f.is_file),
            (12, Direction::Send, false),
            "{b}"
        );
        assert_eq!(
            f.path, "/home/alice/docs",
            "{b}: control characters dropped"
        );
        assert_eq!(f.file_count, 3, "{b}");
        let names: serde_json::Value = serde_json::from_str(&f.files).unwrap();
        assert_eq!(names, json!([["a.txt", 10], ["btxt.exe", 5]]), "{b}");
        assert_eq!(f.ip.as_deref(), Some("198.51.100.4"), "{b}");
        let alarms = c.alarms().await;
        let a = &alarms[0];
        assert_eq!(
            (a.device_rustdesk_id.as_str(), a.typ, a.conn_id),
            (ID, 1, 12),
            "{b}"
        );
        let info: serde_json::Value = serde_json::from_str(&a.info).unwrap();
        assert_eq!(info, json!({ "ip": "198.51.100.4" }), "{b}");
    })
    .await;
}
