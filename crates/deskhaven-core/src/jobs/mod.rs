//! Every delete removes at most one batch per write transaction and pauses between batches: SQLite
//! has a single writer, and the pause outlasts its longest busy-handler sleep, so a waiting writer
//! always gets its turn. A failed table or step is logged and retried by the next run.

mod expiry;
mod purge;
mod retention;

pub use expiry::{ExpiryCleanup, ExpiryReport, NONCE_RETENTION, STRATEGY_KEYS_GRACE};
pub(crate) use purge::{Doomed, Plain};
pub use purge::{PURGE_BATCH, PURGE_PAUSE, Purge};
pub use retention::{
    AuditKind, AuditRetention, RETENTION_ACTION, RETENTION_TARGET, RetentionReport,
};

use std::error::Error;
use std::time::Duration;

use deskhaven_db::time::{self, TimestampMs};
use sea_orm::DbErr;

fn cutoff(now_ms: i64, age: Duration) -> Result<TimestampMs, DbErr> {
    let age = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
    let ms = now_ms.saturating_sub(age);
    time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
}

fn chain(err: &dyn Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoff_subtracts_and_saturates() {
        let day = Duration::from_secs(86_400);

        assert_eq!(
            cutoff(2 * 86_400_000, day).unwrap().timestamp_millis(),
            86_400_000
        );
        assert!(
            cutoff(0, Duration::MAX).is_err(),
            "out of range, not wrapped"
        );
    }

    #[derive(Debug, thiserror::Error)]
    #[error("cleanup failed")]
    struct Outer(#[source] std::io::Error);

    #[test]
    fn chain_lists_every_source() {
        let err = Outer(std::io::Error::other("disk gone"));

        assert_eq!(chain(&err), "cleanup failed: disk gone");
    }
}
