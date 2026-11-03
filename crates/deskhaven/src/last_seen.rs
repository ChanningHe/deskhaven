//! The `last_seen_at` flush task (02 §3, §7): every `devices.last_seen_flush` (hot-reloaded,
//! read before each wait) it writes the presence table's contacts in batches
//! ([`LastSeenFlusher`]). Shutdown stops the periodic loop with the token; [`FlushTask::finish`]
//! then writes what is left, after HTTP and the signal server stopped (so their last contacts
//! are included) and before the database closes.

use std::time::Duration;

use deskhaven_config::settings::DEVICES_LAST_SEEN_FLUSH;
use deskhaven_core::devices::{FlushReport, LastSeenFlusher, Presence};
use deskhaven_core::state::AppState;
use sea_orm::DatabaseConnection;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Bound on [`FlushTask::finish`] (waiting for the loop plus the last flush): its own share of
/// the shutdown deadline (`serve::SHUTDOWN_DEADLINE`).
pub const FINAL_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// The running loop, and what a replacement flusher needs if the loop died.
pub struct FlushTask {
    handle: JoinHandle<LastSeenFlusher>,
    db: DatabaseConnection,
    presence: Presence,
}

impl FlushTask {
    /// The task for `state`, until `token` is cancelled.
    pub fn start(state: &AppState, token: CancellationToken) -> Self {
        let settings = state.settings.clone();
        let interval = move || settings.snapshot().get(&DEVICES_LAST_SEEN_FLUSH).as_std();
        Self::spawn(
            state.db.clone(),
            state.devices.presence().clone(),
            interval,
            token,
        )
    }

    /// Flushes `presence` into `db` every `interval()` until `token` is cancelled.
    pub fn spawn(
        db: DatabaseConnection,
        presence: Presence,
        interval: impl Fn() -> Duration + Send + 'static,
        token: CancellationToken,
    ) -> Self {
        let flusher = LastSeenFlusher::new(db.clone(), presence.clone());
        let handle = tokio::spawn(run(flusher, interval, token));
        Self {
            handle,
            db,
            presence,
        }
    }

    /// Waits for the loop (its token must be cancelled) and flushes once more, within
    /// [`FINAL_FLUSH_TIMEOUT`]; `None` if that ran out.
    pub async fn finish(self) -> Option<FlushReport> {
        let Self {
            handle,
            db,
            presence,
        } = self;
        let last = async move {
            let mut flusher = handle.await.unwrap_or_else(|err| {
                tracing::warn!(error = %err, "last_seen_at flush task failed");
                LastSeenFlusher::new(db, presence)
            });
            flusher.flush().await
        };
        match tokio::time::timeout(FINAL_FLUSH_TIMEOUT, last).await {
            Ok(report) => {
                tracing::info!(
                    written = report.written,
                    failed = report.failed,
                    "last_seen_at flushed"
                );
                Some(report)
            }
            Err(_) => {
                tracing::warn!(timeout = ?FINAL_FLUSH_TIMEOUT, "final last_seen_at flush timed out");
                None
            }
        }
    }
}

async fn run(
    mut flusher: LastSeenFlusher,
    interval: impl Fn() -> Duration,
    token: CancellationToken,
) -> LastSeenFlusher {
    loop {
        tokio::select! {
            // A due flush after the cancellation is left to `finish`.
            biased;
            () = token.cancelled() => return flusher,
            () = tokio::time::sleep(interval()) => {}
        }
        // Cancellation also interrupts a flush in progress, so it never overlaps the final
        // flush or the database close; `flush` is cancel-safe (nothing taken is lost).
        let report = tokio::select! {
            biased;
            () = token.cancelled() => return flusher,
            report = flusher.flush() => report,
        };
        tracing::debug!(
            written = report.written,
            batches = report.batches,
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
            let task = FlushTask::spawn(db.conn.clone(), presence.clone(), || TICK, token.clone());

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
            let report = task.finish().await;

            // Assert
            let b = db.backend;
            assert!(periodic.is_ok(), "{b}: the loop never flushed");
            assert_eq!(report.map(|r| r.written), Some(1), "{b}");
            assert_eq!(last_seen(db).await, T0_MS + 2_000, "{b}: written at finish");
        })
        .await;
    }
}
