//! The `last_seen_at` flush (02 §3, §7): every `devices.last_seen_flush` (hot-reloaded,
//! read before each wait) it writes the presence table's contacts in batches
//! ([`LastSeenFlusher`]). Its [`Job::finish`] writes what is left, after HTTP and the signal
//! server stopped (so their last contacts are included) and before the database closes.

use std::time::Duration;

use deskhaven_config::settings::DEVICES_LAST_SEEN_FLUSH;
use deskhaven_core::devices::{LastSeenFlusher, Presence};
use deskhaven_core::state::AppState;
use sea_orm::DatabaseConnection;

use super::scheduler::Job;

/// The flush job; `I` reads the interval.
pub struct LastSeen<I> {
    flusher: LastSeenFlusher,
    interval: I,
}

/// The job for `state`, on its `devices.last_seen_flush` setting.
pub fn from_state(state: &AppState) -> LastSeen<impl Fn() -> Duration + Send + 'static> {
    let settings = state.settings.clone();
    LastSeen::new(
        state.db.clone(),
        state.devices.presence().clone(),
        move || settings.snapshot().get(&DEVICES_LAST_SEEN_FLUSH).as_std(),
    )
}

impl<I: Fn() -> Duration + Send + 'static> LastSeen<I> {
    /// Flushes `presence` into `db` every `interval()`.
    pub fn new(db: DatabaseConnection, presence: Presence, interval: I) -> Self {
        Self {
            flusher: LastSeenFlusher::new(db, presence),
            interval,
        }
    }
}

impl<I: Fn() -> Duration + Send + 'static> Job for LastSeen<I> {
    const NAME: &'static str = "last_seen_flush";

    fn interval(&self) -> Duration {
        (self.interval)()
    }

    /// `flush` is cancel-safe: nothing taken from the presence table is lost.
    async fn run(&mut self) {
        let report = self.flusher.flush().await;
        tracing::debug!(
            written = report.written,
            batches = report.batches,
            failed = report.failed,
            "last_seen_at flushed"
        );
    }

    async fn finish(&mut self) {
        let report = self.flusher.flush().await;
        tracing::info!(
            written = report.written,
            failed = report.failed,
            "last_seen_at flushed"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Arc;

    use deskhaven_core::clock::ManualClock;
    use deskhaven_core::devices::{DeviceRepository, Registration};
    use deskhaven_db::testing::{TestDb, each_migrated_backend};
    use deskhaven_db::time;
    use tokio_util::sync::CancellationToken;

    use super::super::scheduler::Scheduler;
    use super::*;

    const T0_MS: i64 = 1_700_000_000_000;
    const ID: &str = "123456789";
    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1));
    const TICK: Duration = Duration::from_millis(10);
    const PATIENCE: Duration = Duration::from_secs(5);

    async fn register(db: &TestDb) {
        let registration = Registration {
            rustdesk_id: ID.to_owned(),
            uuid: b"uuid".to_vec(),
            pk: vec![9; 32],
            ip: IP,
            at: time::from_millis(T0_MS).unwrap(),
        };
        let repo = DeviceRepository::new(db.conn.clone());
        repo.upsert_registration(&registration).await.unwrap();
    }

    async fn last_seen(db: &TestDb) -> i64 {
        let repo = DeviceRepository::new(db.conn.clone());
        let row = repo.find_by_rustdesk_id(ID).await.unwrap().unwrap();
        row.last_seen_at.timestamp_millis()
    }

    #[tokio::test]
    async fn the_loop_flushes_periodically_and_finish_writes_the_rest() {
        each_migrated_backend(async |db: &TestDb| {
            // Arrange
            register(db).await;
            let clock = Arc::new(ManualClock::at(T0_MS + 1_000));
            let presence = Presence::new(clock.clone());
            let token = CancellationToken::new();
            let mut scheduler = Scheduler::new(token.clone());
            scheduler.spawn(LastSeen::new(db.conn.clone(), presence.clone(), || TICK));

            // Act: a contact is picked up by the loop on its own.
            presence.heartbeat(ID, IP, &[]);
            let periodic = tokio::time::timeout(PATIENCE, async {
                while last_seen(db).await != T0_MS + 1_000 {
                    tokio::time::sleep(TICK).await;
                }
            })
            .await;
            token.cancel();
            clock.advance(Duration::from_secs(1));
            presence.heartbeat(ID, IP, &[]);
            let done = scheduler.finish(PATIENCE).await;

            // Assert
            let b = db.backend;
            assert!(periodic.is_ok(), "{b}: the loop never flushed");
            assert!(done, "{b}");
            assert_eq!(last_seen(db).await, T0_MS + 2_000, "{b}: written at finish");
        })
        .await;
    }
}
