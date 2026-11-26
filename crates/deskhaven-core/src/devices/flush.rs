//! Rows are sorted by RustDesk id so concurrent writers on PostgreSQL lock them in the same
//! order, and only move forward (`last_seen_at < new`), so a late write never undoes a newer
//! one. Contacts stay here until their batch succeeded.

use std::collections::HashMap;

use deskhaven_db::time;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement, Value};

use super::presence::{DEFAULT_CAPACITY, Presence, Seen};

pub const FLUSH_BATCH: usize = 500;
pub const MAX_RETRY: usize = DEFAULT_CAPACITY;
const PARAMS_PER_ROW: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FlushReport {
    /// Rows already newer count too.
    pub written: usize,
    pub batches: usize,
    pub failed: usize,
}

pub struct LastSeenFlusher {
    db: DatabaseConnection,
    presence: Presence,
    batch: usize,
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

    pub fn with_batch_size(self, batch: usize) -> Self {
        Self {
            batch: batch.max(1),
            ..self
        }
    }

    /// Never fails: errors are logged and the batch is retried. Cancel-safe.
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

/// A NULL ip keeps the stored one.
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

fn placeholder(backend: DbBackend, n: usize) -> String {
    match backend {
        DbBackend::Postgres => format!("${n}"),
        _ => "?".to_owned(),
    }
}
