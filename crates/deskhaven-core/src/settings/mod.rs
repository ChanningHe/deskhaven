//! Settings persistence and hot reload (05-config §3.2-3.3).
//!
//! Validation, pins and snapshots live in `deskhaven_config::settings`;
//! this module loads rows, applies batches in one write transaction with
//! optimistic `version` checks, writes history, encrypts sensitive values
//! and publishes each new snapshot (lock-free reads, `watch` notification).
//!
//! Single writer process (05-config §3.6): snapshots are rebuilt on this
//! process's own updates, and from the locked read of every update attempt
//! (so a conflict also refreshes stale versions); another process writing
//! the same database is not otherwise observed.

mod apply;
mod error;
mod store;

use std::sync::Arc;

use arc_swap::ArcSwap;
use deskhaven_config::settings::{self, Change, LoadWarning, PinSet, Problem, Snapshot};
use deskhaven_db::entity::setting;
use sea_orm::DatabaseConnection;
use tokio::sync::{Mutex, watch};

pub use error::SettingsError;
pub use store::value_context;

use crate::crypto::SecretBox;

/// Longest accepted history note.
pub const MAX_REASON_LEN: usize = 500;

/// One batch of changes, applied atomically. Changes whose value equals
/// the current effective value are skipped (no version bump, no history).
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
    /// Stored rows that could not be used, until each key is rewritten.
    row_warnings: ArcSwap<Vec<LoadWarning>>,
    /// Keeps commit and publish of one batch together, so snapshots are
    /// published in commit order.
    writer: Mutex<()>,
}

impl SettingsService {
    /// Reads every stored row and builds the first snapshot. Rows that
    /// cannot be used are ignored (see [`SettingsService::load_warnings`]);
    /// only database errors fail.
    pub async fn load(
        db: DatabaseConnection,
        crypto: Arc<SecretBox>,
        pins: PinSet,
    ) -> Result<(Self, Vec<LoadWarning>), SettingsError> {
        let rows = store::fetch(&db).await?;
        let (snapshot, row_warnings) = observe(&rows, &crypto, &pins);
        let snapshot = Arc::new(snapshot);
        let (notify, _) = watch::channel(Arc::clone(&snapshot));
        let inner = Inner {
            db,
            crypto,
            pins,
            current: ArcSwap::new(snapshot),
            notify,
            row_warnings: ArcSwap::from_pointee(row_warnings),
            writer: Mutex::new(()),
        };
        let service = Self {
            inner: Arc::new(inner),
        };
        let warnings = service.load_warnings();
        Ok((service, warnings))
    }

    /// The current snapshot. Lock-free; hold it for a consistent view.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.current.load_full()
    }

    /// Receives every snapshot published after a successful update or a
    /// refresh from the database.
    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.inner.notify.subscribe()
    }

    /// Unusable stored rows (kept until the key is successfully rewritten)
    /// and cross-key rules that do not hold on the current snapshot. Never
    /// contains values; for readiness checks and admin banners.
    pub fn load_warnings(&self) -> Vec<LoadWarning> {
        let mut warnings = (*self.inner.row_warnings.load_full()).clone();
        warnings.extend(self.snapshot().rule_warnings());
        warnings
    }

    /// Validates and applies `request` atomically, then publishes the new
    /// snapshot. On any error nothing is written. Cancel-safe: the batch
    /// runs in its own task, so dropping this future cannot leave the
    /// database and the snapshot apart.
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

/// The snapshot of `rows` and the warnings about rows it could not use
/// (cross-key rule warnings are recomputed from the snapshot on demand).
fn observe(
    rows: &[setting::Model],
    crypto: &SecretBox,
    pins: &PinSet,
) -> (Snapshot, Vec<LoadWarning>) {
    let (stored, mut warnings) = store::decode(rows, crypto);
    let (snapshot, build_warnings) = Snapshot::build(pins, stored);
    let rules = snapshot.rule_warnings();
    warnings.extend(build_warnings.into_iter().filter(|w| !rules.contains(w)));
    (snapshot, warnings)
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
