//! The cleanups first run [`CLEANUP_FIRST_DELAY`] after startup, so a server restarted more
//! often than their interval still cleans up. Their intervals are constants: retention works
//! in days and expiry only frees space.

mod last_seen;
mod scheduler;

use std::sync::Arc;
use std::time::Duration;

use deskhaven_core::jobs::{AuditRetention, ExpiryCleanup};
use deskhaven_core::state::AppState;
use tokio_util::sync::CancellationToken;

pub use last_seen::LastSeen;
pub use scheduler::{Job, Scheduler};

/// Its own share of the shutdown deadline.
pub const FINAL_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub const EXPIRY_INTERVAL: Duration = Duration::from_secs(10 * 60);
pub const CLEANUP_FIRST_DELAY: Duration = Duration::from_secs(60);

pub fn start(state: &Arc<AppState>, token: CancellationToken) -> Scheduler {
    let mut scheduler = Scheduler::new(token);
    scheduler.spawn(last_seen::from_state(state));
    scheduler.spawn(AuditRetention::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.clock),
    ));
    scheduler.spawn(ExpiryCleanup::new(Arc::clone(state)));
    scheduler
}

impl Job for AuditRetention {
    const NAME: &'static str = "audit_retention";

    fn interval(&self) -> Duration {
        RETENTION_INTERVAL
    }

    fn first_delay(&self) -> Duration {
        CLEANUP_FIRST_DELAY
    }

    async fn run(&mut self) {
        let report = AuditRetention::run(self).await;
        tracing::debug!(
            deleted = report.total(),
            failed = report.failed.len(),
            "audit retention ran"
        );
    }
}

impl Job for ExpiryCleanup {
    const NAME: &'static str = "expiry_cleanup";

    fn interval(&self) -> Duration {
        EXPIRY_INTERVAL
    }

    fn first_delay(&self) -> Duration {
        CLEANUP_FIRST_DELAY
    }

    async fn run(&mut self) {
        let report = ExpiryCleanup::run(self).await;
        tracing::debug!(
            deleted = report.total(),
            failed = report.failed,
            ?report,
            "expiry cleanup ran"
        );
    }
}
