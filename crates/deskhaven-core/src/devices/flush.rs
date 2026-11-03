//! [`LastSeenFlusher`]: writes `devices.last_seen_at` / `last_ip` from the presence table in
//! batches (02 §3: every `devices.last_seen_flush`), so neither the heartbeat nor the signal
//! server writes per contact.
//!
//! - **Batches**: each flush takes what [`Presence::take_unflushed`] reports (which also
//!   evicts offline entries already flushed) and writes it with one statement per
//!   [`FLUSH_BATCH`] rows, `UPDATE devices … FROM (VALUES …)`, rows sorted by RustDesk id so
//!   concurrent writers on PostgreSQL lock them in the same order. Supported by PostgreSQL
//!   and SQLite ≥ 3.33 (the bundled SQLite is newer).
//! - **Never backwards**: a row only moves forward (`last_seen_at < new`), so a late or
//!   repeated write never undoes a newer value.
//! - **Nothing lost**: contacts stay in the flusher until their batch succeeded. A failed
//!   batch is retried by the next flush, and so is everything a flush did not get to when
//!   its future was dropped (shutdown cancels the periodic flush; the final one writes the
//!   rest). The presence table no longer holds them, so at most [`MAX_RETRY`] devices are
//!   kept (oldest contacts dropped beyond that, logged).

use std::collections::HashMap;

use deskhaven_db::time;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, Value};

use super::presence::{DEFAULT_CAPACITY, Presence, Seen};

/// Rows per statement.
pub const FLUSH_BATCH: usize = 500;
/// Devices kept for retry after failed batches, at most.
pub const MAX_RETRY: usize = DEFAULT_CAPACITY;
/// Bound parameters per row of the `VALUES` list.
const PARAMS_PER_ROW: usize = 3;

/// What one [`LastSeenFlusher::flush`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    /// Devices whose contact was written (rows already newer count too).
    pub written: usize,
    /// Statements that succeeded.
    pub batches: usize,
    /// Devices whose batch failed, kept for the next flush.
    pub failed: usize,
}

/// Owned by the flush task; keeps unwritten contacts between flushes.
pub struct LastSeenFlusher {
    db: DatabaseConnection,
    presence: Presence,
    batch: usize,
    /// Contacts not written yet, newest per device.
    pending: HashMap<String, Seen>,
}

impl LastSeenFlusher {
    pub fn new(db: DatabaseConnection, presence: Presence) -> Self {
        Self {
            db,
            presence,
            batch: FLUSH_BATCH,
            pending: HashMap::new(),
        }
    }

    /// Rows per statement (at least 1); [`FLUSH_BATCH`] by default.
    pub fn with_batch_size(self, batch: usize) -> Self {
        Self {
            batch: batch.max(1),
            ..self
        }
    }

    /// Writes every contact seen since the previous flush, plus those an earlier flush did
    /// not write. Never fails: errors are logged and the batch is retried next time.
    /// Cancel-safe: dropping the future loses nothing.
    pub async fn flush(&mut self) -> FlushReport {
        for seen in self.presence.take_unflushed() {
            keep_newest(&mut self.pending, seen);
        }
        let mut due: Vec<Seen> = self.pending.values().cloned().collect();
        due.sort_unstable_by(|a, b| a.rustdesk_id.cmp(&b.rustdesk_id));
        let mut report = FlushReport::default();
        for batch in due.chunks(self.batch) {
            match write_batch(&self.db, batch).await {
                Ok(()) => {
                    report.written += batch.len();
                    report.batches += 1;
                    self.forget_written(batch);
                }
                Err(err) => {
                    tracing::warn!(error = %err, rows = batch.len(), "last_seen_at flush failed; retrying next time");
                    report.failed += batch.len();
                }
            }
        }
        self.cap_pending();
        report
    }

    /// Drops the written contacts, unless a newer one arrived meanwhile.
    fn forget_written(&mut self, batch: &[Seen]) {
        for seen in batch {
            let same = self
                .pending
                .get(&seen.rustdesk_id)
                .is_some_and(|kept| kept.last_seen_ms == seen.last_seen_ms);
            if same {
                self.pending.remove(&seen.rustdesk_id);
            }
        }
    }

    /// Drops the oldest contacts beyond [`MAX_RETRY`].
    fn cap_pending(&mut self) {
        let excess = self.pending.len().saturating_sub(MAX_RETRY);
        if excess == 0 {
            return;
        }
        let mut by_age: Vec<(i64, String)> = self
            .pending
            .values()
            .map(|s| (s.last_seen_ms, s.rustdesk_id.clone()))
            .collect();
        by_age.sort_unstable();
        for (_, id) in by_age.into_iter().take(excess) {
            self.pending.remove(&id);
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

/// One `UPDATE … FROM (VALUES …)` for `batch` (columns `column1..3` = id, unix ms, ip; a
/// NULL ip keeps the stored one).
async fn write_batch(db: &DatabaseConnection, batch: &[Seen]) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let mut values = Vec::with_capacity(batch.len() * PARAMS_PER_ROW);
    let mut rows = Vec::with_capacity(batch.len());
    for seen in batch
        .iter()
        .filter(|s| time::from_millis(s.last_seen_ms).is_some())
    {
        let n = values.len();
        rows.push(format!(
            "(CAST({} AS TEXT), CAST({} AS BIGINT), CAST({} AS TEXT))",
            placeholder(backend, n + 1),
            placeholder(backend, n + 2),
            placeholder(backend, n + 3),
        ));
        values.push(Value::from(seen.rustdesk_id.clone()));
        values.push(Value::from(seen.last_seen_ms));
        values.push(Value::from(seen.ip.map(|ip| ip.to_string())));
    }
    if rows.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE devices SET last_seen_at = v.column2, \
         last_ip = COALESCE(v.column3, devices.last_ip) \
         FROM (VALUES {}) AS v \
         WHERE devices.rustdesk_id = v.column1 AND devices.last_seen_at < v.column2",
        rows.join(", ")
    );
    db.execute_raw(Statement::from_sql_and_values(backend, sql, values))
        .await?;
    Ok(())
}

/// The `n`-th (1-based) bound parameter in `backend`'s syntax.
fn placeholder(backend: DbBackend, n: usize) -> String {
    match backend {
        DbBackend::Postgres => format!("${n}"),
        _ => "?".to_owned(),
    }
}
