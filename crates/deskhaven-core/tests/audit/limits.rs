//! Per-device quota (06 §2.4: by device id as well as by source IP), checked after the
//! device is verified, so a stranger cannot spend a real device's quota.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::{AUDIT_PER_DEVICE_PER_MINUTE, Envelope, Ingest, Record};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{ID, closed, ctx, envelope, uuid_b64};

/// A cheap record: closes of distinct connections.
fn record(i: u32) -> Record {
    closed(i64::from(i), 1)
}

#[tokio::test]
async fn a_device_over_its_quota_is_told_to_wait_and_others_are_not() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.register("555555555").await;
        for i in 0..AUDIT_PER_DEVICE_PER_MINUTE {
            assert_eq!(c.send(&format!("n{i}"), record(i)).await, Ingest::Stored);
        }

        // Act
        let over = c.send("over", record(u32::MAX)).await;
        let other = c.send_from("555555555", "other", record(0)).await;
        c.clock.advance(Duration::from_secs(60));
        let later = c.send("later", record(u32::MAX)).await;

        // Assert
        let b = db.backend;
        let Ingest::TooFrequent { retry_after } = over else {
            panic!("{b}: {over:?}");
        };
        assert!(retry_after > Duration::ZERO, "{b}");
        assert_eq!((other, later), (Ingest::Stored, Ingest::Stored), "{b}");
        assert_eq!(
            c.nonces().await,
            u64::from(AUDIT_PER_DEVICE_PER_MINUTE) + 2,
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn records_with_a_wrong_uuid_do_not_spend_the_devices_quota() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let forged = Envelope {
            uuid: uuid_b64("someone-else"),
            ..envelope("forged")
        };
        for i in 0..=AUDIT_PER_DEVICE_PER_MINUTE {
            let outcome = c.audit.ingest(forged.clone(), record(i)).await.unwrap();
            assert_eq!(outcome, Ingest::NotFound);
        }

        // Act
        let real = c.send("real", record(0)).await;

        // Assert
        assert_eq!(real, Ingest::Stored, "{}: {ID}", db.backend);
    })
    .await;
}
