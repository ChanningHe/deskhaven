//! The `last_seen_at` batch flush (M2-14, 02 §3): presence contacts reach the registry in
//! batches, never backwards, failed batches are retried, flushed offline entries are evicted.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use deskhaven_core::devices::{FlushReport, HEARTBEAT_TTL, LastSeenFlusher};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, IntoActiveModel};

use super::support::{Ctx, T0_MS, ctx};

const IDS: [&str; 5] = [
    "100000001",
    "100000002",
    "100000003",
    "100000004",
    "100000005",
];

fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(198, 51, 100, last))
}

impl Ctx {
    fn flusher(&self, batch: usize) -> LastSeenFlusher {
        LastSeenFlusher::new(self.conn.clone(), self.devices.presence().clone())
            .with_batch_size(batch)
    }

    async fn last_seen(&self, rustdesk_id: &str) -> (i64, String) {
        let row = self.row(rustdesk_id).await;
        (row.last_seen_at.timestamp_millis(), row.last_ip)
    }
}

async fn rename_devices(db: &TestDb, from: &str, to: &str) {
    let sql = format!("ALTER TABLE {from} RENAME TO {to}");
    db.conn.execute_unprepared(&sql).await.unwrap();
}

#[tokio::test]
async fn contacts_are_written_in_batches_and_flushed_offline_entries_evicted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        for id in IDS {
            c.device(id).await;
        }
        c.clock.advance(Duration::from_secs(3));
        for (n, id) in (1..).zip(IDS) {
            c.devices.presence().heartbeat(id, ip(n), &[]);
        }
        let mut flusher = c.flusher(2);

        // Act
        let first = flusher.flush().await;
        let tracked_while_online = c.devices.presence().len();
        c.clock.advance(HEARTBEAT_TTL);
        let idle = flusher.flush().await;

        // Assert
        let b = db.backend;
        let expected = FlushReport {
            written: 5,
            batches: 3,
            failed: 0,
        };
        assert_eq!(first, expected, "{b}");
        for (n, id) in (1..).zip(IDS) {
            let written = c.last_seen(id).await;
            assert_eq!(written, (T0_MS + 3_000, ip(n).to_string()), "{b}: {id}");
        }
        assert_eq!(tracked_while_online, 5, "{b}: online entries stay");
        assert_eq!(idle, FlushReport::default(), "{b}");
        assert_eq!(c.devices.presence().len(), 0, "{b}: offline and flushed");
    })
    .await;
}

#[tokio::test]
async fn last_seen_never_moves_backwards() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(IDS[0]).await;
        let mut row = c.row(IDS[0]).await.into_active_model();
        row.last_seen_at = Set(time::from_millis(T0_MS + 60_000).unwrap());
        row.update(&c.conn).await.unwrap();
        c.devices.presence().heartbeat(IDS[0], ip(9), &[]);

        // Act
        let report = c.flusher(10).flush().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.written, 1, "{b}");
        let (seen, last_ip) = c.last_seen(IDS[0]).await;
        assert_eq!(seen, T0_MS + 60_000, "{b}");
        assert_ne!(
            last_ip,
            ip(9).to_string(),
            "{b}: an older contact changes nothing"
        );
    })
    .await;
}

/// The presence table has already evicted an offline device by the time its write fails;
/// the flusher keeps the contact and writes it next time.
#[tokio::test]
async fn a_failed_batch_is_written_by_the_next_flush() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(IDS[0]).await;
        c.clock.advance(Duration::from_secs(3));
        c.devices.presence().heartbeat(IDS[0], ip(7), &[]);
        c.clock.advance(HEARTBEAT_TTL);
        let mut flusher = c.flusher(10);
        rename_devices(db, "devices", "devices_away").await;

        // Act
        let failed = flusher.flush().await;
        let evicted = c.devices.presence().is_empty();
        rename_devices(db, "devices_away", "devices").await;
        let retried = flusher.flush().await;

        // Assert
        let b = db.backend;
        assert_eq!(failed.failed, 1, "{b}");
        assert!(evicted, "{b}");
        assert_eq!(retried.written, 1, "{b}");
        let written = c.last_seen(IDS[0]).await;
        assert_eq!(written, (T0_MS + 3_000, ip(7).to_string()), "{b}");
    })
    .await;
}
