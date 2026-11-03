//! The work of the periodic background jobs (02 §1, M2-20); the scheduler that runs them
//! lives in the server binary (`deskhaven::jobs`), the `last_seen_at` flush in
//! [`devices`](crate::devices::LastSeenFlusher).
//!
//! - [`AuditRetention`]: deletes audit records older than `audit.retention.*` (per type, `0`
//!   keeps them forever) and records each deleting batch in `console_audit` (06 §7), with
//!   one immutable row inserted in that batch's transaction.
//! - [`ExpiryCleanup`]: drops state that has expired or can no longer matter: audit nonces
//!   (after [`NONCE_RETENTION`]), finished setup tokens, `device_strategy_keys` of deleted
//!   devices (after [`STRATEGY_KEYS_GRACE`]), expired OIDC flows and finished login
//!   challenges, and, through their services, dead sessions and stale lockout counters.
//!
//! **Batches** ([`Purge`]): every delete here removes at most one batch of rows per write
//! transaction (`BEGIN IMMEDIATE` on SQLite), selected with `FOR UPDATE SKIP LOCKED` on
//! PostgreSQL so it never waits for a row another writer holds, and pauses between batches.
//! SQLite has a single writer: a long purge must not hold the write lock, and the pause is
//! longer than SQLite's longest busy-handler sleep, so a waiting writer always gets its turn.
//!
//! **Time** comes from the injected clock, so tests move it instead of sleeping.
//!
//! **Failures** never stop a run early: each table (retention) or step (expiry) is logged
//! and retried by the next run, and the others still run. A run dropped at any await (the
//! scheduler stops on shutdown) loses at most its current, uncommitted batch.

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

/// `now_ms` minus `age`, as a stored timestamp.
fn cutoff(now_ms: i64, age: Duration) -> Result<TimestampMs, DbErr> {
    let age = i64::try_from(age.as_millis()).unwrap_or(i64::MAX);
    let ms = now_ms.saturating_sub(age);
    time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
}

/// `err` and its sources, for a log line.
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
