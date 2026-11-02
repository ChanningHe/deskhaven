//! Settings persistence and hot reload (05-config §3.2-3.3).
//!
//! Validation, pins and snapshots live in `deskhaven_config::settings`;
//! this module loads rows, applies batches in one write transaction with
//! optimistic `version` checks, writes history, encrypts sensitive values
//! and publishes each new snapshot (lock-free reads, `watch` notification).

mod error;
mod store;

use std::sync::Arc;

use arc_swap::ArcSwap;
use deskhaven_config::Source;
use deskhaven_config::settings::{self, Accepted, Change, LoadWarning, PinSet, Snapshot, Update};
use deskhaven_db::entity::setting;
use deskhaven_db::{begin_write, time};
use sea_orm::DatabaseConnection;
use serde_json::Value as Json;
use tokio::sync::{Mutex, watch};

pub use error::SettingsError;
pub use store::value_context;

use crate::crypto::SecretBox;
use store::Stamp;

/// One batch of changes, applied atomically.
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub changes: Vec<Change>,
    /// Acting user; `None` for the system.
    pub actor: Option<i64>,
    /// Free-form note stored in the history.
    pub reason: Option<String>,
}

/// Owns the effective settings of the process.
pub struct SettingsService {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    pins: PinSet,
    current: ArcSwap<Snapshot>,
    notify: watch::Sender<Arc<Snapshot>>,
    /// Keeps commit and publish of one batch together, so snapshots are
    /// published in commit order.
    writer: Mutex<()>,
}

impl SettingsService {
    /// Reads every stored row and builds the first snapshot. Rows that
    /// cannot be used are ignored and returned as warnings for the caller
    /// to log; only database errors fail.
    pub async fn load(
        db: DatabaseConnection,
        crypto: Arc<SecretBox>,
        pins: PinSet,
    ) -> Result<(Self, Vec<LoadWarning>), SettingsError> {
        let rows = store::fetch(&db).await?;
        let (stored, mut warnings) = store::decode(&rows, &crypto);
        let (snapshot, rule_warnings) = Snapshot::build(&pins, stored);
        warnings.extend(rule_warnings);
        let snapshot = Arc::new(snapshot);
        let (notify, _) = watch::channel(Arc::clone(&snapshot));
        let service = Self {
            db,
            crypto,
            pins,
            current: ArcSwap::new(snapshot),
            notify,
            writer: Mutex::new(()),
        };
        Ok((service, warnings))
    }

    /// The current snapshot. Lock-free; hold it for a consistent view.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }

    /// Receives every snapshot published after a successful update.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.notify.subscribe()
    }

    /// Validates and applies `request` atomically, then publishes the new
    /// snapshot. On any error nothing is written or published.
    pub async fn update(&self, request: UpdateRequest) -> Result<Arc<Snapshot>, SettingsError> {
        let accepted = settings::prepare(&self.pins, &request.changes)?;
        if accepted.is_empty() {
            return Ok(self.snapshot());
        }
        let _writer = self.writer.lock().await;
        let next = Arc::new(self.commit(&accepted, &request).await?);
        self.current.store(Arc::clone(&next));
        self.notify.send_replace(Arc::clone(&next));
        Ok(next)
    }

    /// Checks versions and cross-key rules against the committed state (not
    /// the in-memory snapshot) inside the write transaction.
    async fn commit(
        &self,
        accepted: &[Accepted],
        request: &UpdateRequest,
    ) -> Result<Snapshot, SettingsError> {
        let tx = begin_write(&self.db).await?;
        store::lock(&tx).await?;
        let rows = store::fetch(&tx).await?;
        check_versions(&rows, accepted)?;
        let (stored, _) = store::decode(&rows, &self.crypto);
        let (current, _) = Snapshot::build(&self.pins, stored);
        let stamp = Stamp {
            by: request.actor,
            at: time::now(),
            reason: request.reason.as_deref(),
        };
        let candidate = current.with_updates(accepted.iter().map(|a| updated(a, &stamp)));
        let changed: Vec<&str> = accepted.iter().map(Accepted::key).collect();
        settings::check_rules(&candidate, &changed)?;
        for change in accepted {
            let old =
                row(&rows, change.key()).map(|row| (row, public_value(&current, change.key())));
            store::write(&tx, &self.crypto, change, old, &stamp).await?;
        }
        tx.commit().await?;
        Ok(candidate)
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

fn updated(accepted: &Accepted, stamp: &Stamp<'_>) -> Update {
    Update {
        key: accepted.key(),
        value: accepted.value().clone(),
        source: Source::Database {
            version: accepted.expected_version() + 1,
            by: stamp.by,
            at: stamp.at.timestamp_millis(),
        },
    }
}

fn public_value(snapshot: &Snapshot, key: &str) -> Json {
    snapshot
        .entry(key)
        .map_or(Json::Null, |e| e.value().to_json())
}
