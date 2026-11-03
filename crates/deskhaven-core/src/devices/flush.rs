//! [`LastSeenFlusher`]: writes `devices.last_seen_at` / `last_ip` from the presence table in
//! batches (02 §3: every `devices.last_seen_flush`), so neither the heartbeat nor the signal
//! server writes per contact.
//!
//! Each flush takes what [`Presence::take_unflushed`] reports (which also evicts offline
//! entries already flushed) and writes it in transactions of at most [`FLUSH_BATCH`] rows.
//! A row only moves forward (`last_seen_at < new`), so a late or repeated flush never undoes
//! a newer value. A batch that fails is kept and merged into the next flush (newest contact
//! per device; at most [`MAX_RETRY`] devices, beyond that the oldest contacts are dropped and
//! logged), because the presence table no longer holds it.

use std::collections::HashMap;

use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, Entity as Devices};
use deskhaven_db::time;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};

use super::presence::{DEFAULT_CAPACITY, Presence, Seen};

/// Rows per transaction.
pub const FLUSH_BATCH: usize = 500;
/// Devices kept for retry after failed batches, at most.
pub const MAX_RETRY: usize = DEFAULT_CAPACITY;

/// What one [`LastSeenFlusher::flush`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    /// Devices whose contact was written (rows already newer count too).
    pub written: usize,
    /// Transactions committed.
    pub batches: usize,
    /// Devices whose batch failed, kept for the next flush.
    pub failed: usize,
}

/// Owned by the flush task; keeps the failed batches between flushes.
pub struct LastSeenFlusher {
    db: DatabaseConnection,
    presence: Presence,
    batch: usize,
    retry: HashMap<String, Seen>,
}

impl LastSeenFlusher {
    pub fn new(db: DatabaseConnection, presence: Presence) -> Self {
        Self {
            db,
            presence,
            batch: FLUSH_BATCH,
            retry: HashMap::new(),
        }
    }

    /// Rows per transaction (at least 1); [`FLUSH_BATCH`] by default.
    pub fn with_batch_size(self, batch: usize) -> Self {
        Self {
            batch: batch.max(1),
            ..self
        }
    }

    /// Writes every contact seen since the previous flush, plus the failed ones before it.
    /// Never fails: errors are logged and the batch is retried next time.
    pub async fn flush(&mut self) -> FlushReport {
        let mut due = std::mem::take(&mut self.retry);
        for seen in self.presence.take_unflushed() {
            keep_newest(&mut due, seen);
        }
        let due: Vec<Seen> = due.into_values().collect();
        let mut report = FlushReport::default();
        for batch in due.chunks(self.batch) {
            match write_batch(&self.db, batch).await {
                Ok(()) => {
                    report.written += batch.len();
                    report.batches += 1;
                }
                Err(err) => {
                    tracing::warn!(error = %err, rows = batch.len(), "last_seen_at flush failed; retrying next time");
                    report.failed += batch.len();
                    for seen in batch {
                        keep_newest(&mut self.retry, seen.clone());
                    }
                }
            }
        }
        self.cap_retry();
        report
    }

    /// Drops the oldest contacts beyond [`MAX_RETRY`].
    fn cap_retry(&mut self) {
        let excess = self.retry.len().saturating_sub(MAX_RETRY);
        if excess == 0 {
            return;
        }
        let mut by_age: Vec<(i64, String)> = self
            .retry
            .values()
            .map(|s| (s.last_seen_ms, s.rustdesk_id.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, id) in by_age.into_iter().take(excess) {
            self.retry.remove(&id);
        }
        tracing::warn!(
            dropped = excess,
            "last_seen_at retry buffer full; oldest contacts dropped"
        );
    }
}

fn keep_newest(due: &mut HashMap<String, Seen>, seen: Seen) {
    let newer = due
        .get(&seen.rustdesk_id)
        .is_none_or(|known| known.last_seen_ms < seen.last_seen_ms);
    if newer {
        due.insert(seen.rustdesk_id.clone(), seen);
    }
}

async fn write_batch(db: &DatabaseConnection, batch: &[Seen]) -> Result<(), DbErr> {
    let tx = begin_write(db).await?;
    for seen in batch {
        let Some(at) = time::from_millis(seen.last_seen_ms) else {
            continue;
        };
        let mut update =
            Devices::update_many().col_expr(device::Column::LastSeenAt, Expr::value(at));
        if let Some(ip) = seen.ip {
            update = update.col_expr(device::Column::LastIp, Expr::value(ip.to_string()));
        }
        update
            .filter(device::Column::RustdeskId.eq(&seen.rustdesk_id))
            .filter(device::Column::LastSeenAt.lt(at))
            .exec(&tx)
            .await?;
    }
    tx.commit().await
}
