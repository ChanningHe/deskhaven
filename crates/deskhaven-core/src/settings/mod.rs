//! Settings persistence and hot reload (05-config §3.2-3.3).
//!
//! Validation, pins and snapshots live in `deskhaven_config::settings`;
//! this module loads rows, applies batches in one write transaction with
//! optimistic `version` checks, writes history, encrypts sensitive values
//! and publishes each new snapshot (lock-free reads, `watch` notification).
//!
//! Single writer process: snapshots are rebuilt only after this process's
//! own updates (or on restart); another process writing the same database
//! is not observed. Multi-instance deployments need a refresh mechanism.

mod error;
mod store;

use std::sync::Arc;

use arc_swap::ArcSwap;
use deskhaven_config::settings::{self, Accepted, Change, LoadWarning, PinSet, Problem, Snapshot};
use deskhaven_db::entity::setting;
use deskhaven_db::{begin_write, time};
use sea_orm::DatabaseConnection;
use serde_json::Value as Json;
use tokio::sync::{Mutex, watch};

pub use error::SettingsError;
pub use store::value_context;

use crate::crypto::SecretBox;
use store::Stamp;

/// Longest accepted history note.
pub const MAX_REASON_LEN: usize = 500;

/// One batch of changes, applied atomically.
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub changes: Vec<Change>,
    /// Acting user; `None` for the system.
    pub actor: Option<i64>,
    /// Free-form note stored in the history (at most [`MAX_REASON_LEN`]
    /// characters, no control characters).
    pub reason: Option<String>,
}

/// Owns the effective settings of the process. Cheap to clone (shared state).
#[derive(Clone)]
pub struct SettingsService {
    inner: Arc<Inner>,
}

struct Inner {
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
        let (snapshot, build_warnings) = Snapshot::build(&pins, stored);
        warnings.extend(build_warnings);
        let snapshot = Arc::new(snapshot);
        let (notify, _) = watch::channel(Arc::clone(&snapshot));
        let inner = Inner {
            db,
            crypto,
            pins,
            current: ArcSwap::new(snapshot),
            notify,
            writer: Mutex::new(()),
        };
        Ok((
            Self {
                inner: Arc::new(inner),
            },
            warnings,
        ))
    }

    /// The current snapshot. Lock-free; hold it for a consistent view.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.current.load_full()
    }

    /// Receives every snapshot published after a successful update.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.inner.notify.subscribe()
    }

    /// Validates and applies `request` atomically, then publishes the new
    /// snapshot. On any error nothing is written or published. Cancel-safe:
    /// commit and publish run in their own task, so dropping this future
    /// cannot leave the database and the snapshot apart.
    pub async fn update(&self, request: UpdateRequest) -> Result<Arc<Snapshot>, SettingsError> {
        check_reason(request.reason.as_deref())?;
        let accepted = settings::prepare(&self.inner.pins, &request.changes)?;
        if accepted.is_empty() {
            return Ok(self.snapshot());
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move { inner.apply(&accepted, &request).await })
            .await
            .map_err(|_| SettingsError::Aborted)?
    }
}

impl Inner {
    async fn apply(
        &self,
        accepted: &[Accepted],
        request: &UpdateRequest,
    ) -> Result<Arc<Snapshot>, SettingsError> {
        let _writer = self.writer.lock().await;
        let next = Arc::new(self.commit(accepted, request).await?);
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
        let candidate = current.with_accepted(accepted, stamp.by, stamp.at.timestamp_millis());
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

fn check_reason(reason: Option<&str>) -> Result<(), SettingsError> {
    let Some(reason) = reason else {
        return Ok(());
    };
    let problem = if reason.chars().count() > MAX_REASON_LEN {
        format!("must be at most {MAX_REASON_LEN} characters")
    } else if reason.chars().any(char::is_control) {
        "must not contain control characters".to_owned()
    } else {
        return Ok(());
    };
    Err(SettingsError::Invalid {
        problems: vec![Problem {
            key: "reason".to_owned(),
            reason: problem,
        }],
    })
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

fn public_value(snapshot: &Snapshot, key: &str) -> Json {
    snapshot
        .entry(key)
        .map_or(Json::Null, |e| e.value().to_json())
}
