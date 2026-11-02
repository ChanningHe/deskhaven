//! One update batch: locked read, refresh, checks, writes, commit, publish.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::{self, Accepted, Snapshot};
use deskhaven_db::entity::setting;
use deskhaven_db::{begin_write, time};
use sea_orm::DatabaseTransaction;

use super::store::{self, Stamp};
use super::{Inner, SettingsError, observe};

/// Upper bound from `BEGIN` to the last write of a batch (the writer lock
/// is held meanwhile). Commit itself is not cut off, so a batch that has
/// committed is always published.
const TX_TIMEOUT: Duration = Duration::from_secs(10);

/// A batch with all rows written, waiting for commit.
struct Staged {
    tx: DatabaseTransaction,
    next: Snapshot,
    written: Vec<&'static str>,
}

impl Inner {
    pub(super) async fn apply(
        &self,
        accepted: &[Accepted],
        request: &super::UpdateRequest,
    ) -> Result<Arc<Snapshot>, SettingsError> {
        let _writer = self.writer.lock().await;
        let staged = tokio::time::timeout(TX_TIMEOUT, self.stage(accepted, request))
            .await
            .map_err(|_| SettingsError::Timeout)??;
        let Some(Staged { tx, next, written }) = staged else {
            return Ok(self.current.load_full());
        };
        tx.commit().await?;
        let remaining = self
            .row_warnings
            .load()
            .iter()
            .filter(|w| !written.contains(&w.key.as_str()))
            .cloned()
            .collect();
        self.row_warnings.store(Arc::new(remaining));
        let next = Arc::new(next);
        self.publish(Arc::clone(&next));
        Ok(next)
    }

    /// Checks versions and cross-key rules against the committed state (not
    /// the in-memory snapshot) inside the write transaction. `None`: every
    /// change is a no-op.
    async fn stage(
        &self,
        accepted: &[Accepted],
        request: &super::UpdateRequest,
    ) -> Result<Option<Staged>, SettingsError> {
        let tx = begin_write(&self.db).await?;
        store::lock(&tx).await?;
        let rows = store::fetch(&tx).await?;
        let current = self.refresh(&rows);
        check_versions(&rows, accepted)?;
        let changes: Vec<Accepted> = accepted
            .iter()
            .filter(|a| changes_value(&current, a))
            .cloned()
            .collect();
        if changes.is_empty() {
            return Ok(None);
        }
        let stamp = Stamp {
            by: request.actor,
            at: time::now(),
            reason: request.reason.as_deref(),
        };
        let next = current.with_accepted(&changes, stamp.by, stamp.at.timestamp_millis());
        let written: Vec<&'static str> = changes.iter().map(Accepted::key).collect();
        settings::check_rules(&next, &written)?;
        for change in &changes {
            let old = row(&rows, change.key()).map(|row| (row, previous(&current, change.key())));
            store::write(&tx, &self.crypto, change, old, &stamp).await?;
        }
        Ok(Some(Staged { tx, next, written }))
    }

    /// Rebuilds the snapshot from the locked read and publishes it if it
    /// differs from memory (another writer, a restore), so even a rejected
    /// batch leaves callers with fresh versions.
    fn refresh(&self, rows: &[setting::Model]) -> Arc<Snapshot> {
        let (snapshot, row_warnings) = observe(rows, &self.crypto, &self.pins);
        self.row_warnings.store(Arc::new(row_warnings));
        let snapshot = Arc::new(snapshot);
        if **self.current.load() != *snapshot {
            self.publish(Arc::clone(&snapshot));
        }
        snapshot
    }

    fn publish(&self, next: Arc<Snapshot>) {
        self.current.store(Arc::clone(&next));
        self.notify.send_replace(next);
    }
}

fn row<'a>(rows: &'a [setting::Model], key: &str) -> Option<&'a setting::Model> {
    rows.iter().find(|row| row.key == key)
}

/// Every key whose stored version (0 = no row) differs from the expected one.
fn check_versions(rows: &[setting::Model], accepted: &[Accepted]) -> Result<(), SettingsError> {
    let stale: Vec<String> = accepted
        .iter()
        .filter(|a| row(rows, a.key()).map_or(0, |r| r.version) != a.expected_version())
        .map(|a| a.key().to_owned())
        .collect();
    match stale.is_empty() {
        true => Ok(()),
        false => Err(SettingsError::VersionConflict { keys: stale }),
    }
}

/// A rewrite of an unusable row always counts, even with the same value.
fn changes_value(current: &Snapshot, change: &Accepted) -> bool {
    current
        .entry(change.key())
        .is_none_or(|e| e.is_fallback() || e.value() != change.value())
}

/// Public JSON text of the previous effective value for the history:
/// `null` when the row was unusable (secrets render as `{"set": bool}`).
fn previous(current: &Snapshot, key: &str) -> String {
    match current.entry(key) {
        Some(entry) if !entry.is_fallback() => entry.value().to_json().to_string(),
        _ => serde_json::Value::Null.to_string(),
    }
}
