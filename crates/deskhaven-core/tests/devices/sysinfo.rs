//! Sysinfo upsert: content hash, no write when unchanged, identity check.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::{
    MAX_FIELD_CHARS, Presets, SYSINFO_PER_IP_PER_MINUTE, Sysinfo, SysinfoOutcome, SysinfoReport,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{ID, b64, ctx, report, sysinfo};

fn plain(info: Sysinfo) -> SysinfoReport {
    report(ID, info, Presets::default())
}

#[tokio::test]
async fn the_first_report_stores_every_field_and_the_hash() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let info = sysinfo("office-pc");

        // Act
        let outcome = c.report(plain(info.clone())).await;

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        let stored = (
            row.hostname,
            row.username,
            row.os,
            row.cpu,
            row.memory,
            row.version,
        );
        let sent = (
            info.hostname,
            info.username,
            info.os,
            info.cpu,
            info.memory,
            info.version,
        );
        assert_eq!(stored, sent, "{b}");
        assert_eq!(row.sysinfo_hash, Some(sysinfo("office-pc").hash()), "{b}");
    })
    .await;
}

/// The same content again writes nothing: the row (including `updated_at`) stays as it was.
#[tokio::test]
async fn an_unchanged_report_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.report(plain(sysinfo("office-pc"))).await;
        let before = c.row(ID).await;
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Act
        let outcome = c.report(plain(sysinfo("office-pc"))).await;

        // Assert
        assert_eq!(outcome, SysinfoOutcome::Unchanged, "{}", db.backend);
        assert_eq!(c.row(ID).await, before, "{}", db.backend);
    })
    .await;
}

/// A changed report replaces the stored fields; a field the client no longer sends (the
/// Windows username before login) becomes empty.
#[tokio::test]
async fn a_changed_report_replaces_the_fields_and_clears_absent_ones() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.report(plain(sysinfo("office-pc"))).await;
        c.next_write_window();
        let changed = Sysinfo {
            username: None,
            ..sysinfo("renamed-pc")
        };

        // Act
        let outcome = c.report(plain(changed.clone())).await;

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        assert_eq!(row.hostname.as_deref(), Some("renamed-pc"), "{b}");
        assert_eq!(row.username, None, "{b}");
        assert_eq!(row.sysinfo_hash, Some(changed.hash()), "{b}");
    })
    .await;
}

#[tokio::test]
async fn unknown_ids_and_mismatched_uuids_are_not_found_and_write_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let before = c.row(ID).await;
        let wrong_uuid = SysinfoReport {
            uuid: b64(b"another-uuid"),
            ..plain(sysinfo("office-pc"))
        };

        // Act
        let unknown = c.report(report("555555555", sysinfo("x"), Presets::default()));
        let unknown = unknown.await;
        let mismatch = c.report(wrong_uuid).await;

        // Assert
        let b = db.backend;
        assert_eq!(unknown, SysinfoOutcome::NotFound, "{b}");
        assert_eq!(mismatch, SysinfoOutcome::NotFound, "{b}");
        assert_eq!(c.row(ID).await, before, "{b}");
    })
    .await;
}

/// Self-reported text is stored without control characters (PostgreSQL TEXT cannot hold
/// NUL) and cut to the field limit.
#[tokio::test]
async fn reported_text_is_sanitized_before_it_is_stored() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let hostile = Sysinfo {
            hostname: Some("pc\u{0}\u{1b}[31m".to_owned()),
            os: Some("x".repeat(MAX_FIELD_CHARS * 4)),
            ..Default::default()
        };

        // Act
        let outcome = c.report(plain(hostile)).await;

        // Assert
        let b = db.backend;
        let row = c.row(ID).await;
        assert_eq!(outcome, SysinfoOutcome::Stored, "{b}");
        assert_eq!(row.hostname.as_deref(), Some("pc[31m"), "{b}");
        assert_eq!(row.os.unwrap().len(), MAX_FIELD_CHARS, "{b}");
    })
    .await;
}

/// At most one changing write per device per interval; unchanged uploads are not limited,
/// and the next window stores the change.
#[tokio::test]
async fn a_second_changing_write_within_the_interval_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.device("987654321").await;
        c.report(plain(sysinfo("one"))).await;

        // Act
        let too_soon = c.report(plain(sysinfo("two"))).await;
        let unchanged = c.report(plain(sysinfo("one"))).await;
        let other = c.report(report("987654321", sysinfo("x"), Presets::default()));
        let other = other.await;
        let hostname_meanwhile = c.row(ID).await.hostname;
        c.next_write_window();
        let later = c.report(plain(sysinfo("two"))).await;

        // Assert
        let b = db.backend;
        assert_eq!(too_soon, SysinfoOutcome::TooFrequent, "{b}");
        assert_eq!(unchanged, SysinfoOutcome::Unchanged, "{b}");
        assert_eq!(other, SysinfoOutcome::Stored, "{b}: per device");
        assert_eq!(hostname_meanwhile.as_deref(), Some("one"), "{b}");
        assert_eq!(later, SysinfoOutcome::Stored, "{b}");
        assert_eq!(c.row(ID).await.hostname.as_deref(), Some("two"), "{b}");
    })
    .await;
}

/// The per-IP bucket of the sysinfo endpoint has its own quota, not the login budget.
#[tokio::test]
async fn sysinfo_requests_are_limited_per_client_network() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let client = |last: u8| ClientAddr {
            addr: IpAddr::V4(Ipv4Addr::new(192, 0, 2, last)),
            is_proxy_fallback: false,
        };

        // Act
        let admitted = (0..SYSINFO_PER_IP_PER_MINUTE)
            .filter(|_| c.devices.admit_sysinfo(client(1)).is_ok())
            .count();
        let over = c.devices.admit_sysinfo(client(1));
        let neighbour = c.devices.admit_sysinfo(client(2));

        // Assert
        let b = db.backend;
        assert_eq!(admitted, SYSINFO_PER_IP_PER_MINUTE as usize, "{b}");
        assert!(over.is_err(), "{b}");
        assert!(neighbour.is_ok(), "{b}");
    })
    .await;
}
