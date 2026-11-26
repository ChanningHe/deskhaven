//! Single writer process: snapshots are rebuilt on this process's own updates and from the locked
//! read of every update attempt; another process writing the same database is not otherwise
//! observed.

mod apply;
mod changes;
mod error;
mod history;
mod store;

use std::sync::Arc;

use arc_swap::ArcSwap;
use deskhaven_config::settings::{self, Change, LoadWarning, PinSet, Problem, Reset, Snapshot};
use deskhaven_db::entity::setting;
use sea_orm::DatabaseConnection;
use tokio::sync::{Mutex, watch};

pub use changes::{AppliedChanges, SettingChange};
pub use error::SettingsError;
pub use history::HistoryEntry;
pub use store::value_context;

use crate::console_audit::TxGuard;
use crate::crypto::SecretBox;
use crate::text;

pub const MAX_REASON_LEN: usize = 500;

/// Changes equal to the current effective value are skipped (no version bump, no history).
#[derive(Debug, Clone)]
pub struct UpdateRequest {
    pub changes: Vec<Change>,
    pub actor: Option<i64>,
    pub reason: Option<String>,
}

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
    row_warnings: ArcSwap<Vec<LoadWarning>>,
    /// Snapshots are published in commit order.
    writer: Mutex<()>,
}

impl SettingsService {
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

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.current.load_full()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<Snapshot>> {
        self.inner.notify.subscribe()
    }

    pub fn load_warnings(&self) -> Vec<LoadWarning> {
        let mut warnings = (*self.inner.row_warnings.load_full()).clone();
        warnings.extend(self.snapshot().rule_warnings());
        warnings
    }

    /// Cancel-safe: the batch runs in its own task, so dropping this future cannot leave the database
    /// and the snapshot apart.
    pub async fn update(&self, request: UpdateRequest) -> Result<Arc<Snapshot>, SettingsError> {
        self.update_guarded(request, Vec::new(), TxGuard::none())
            .await
    }

    pub async fn update_guarded(
        &self,
        request: UpdateRequest,
        resets: Vec<Reset>,
        guard: TxGuard<AppliedChanges>,
    ) -> Result<Arc<Snapshot>, SettingsError> {
        check_reason(request.reason.as_deref())?;
        let accepted = settings::prepare_batch(&self.inner.pins, &request.changes, &resets)?;
        if accepted.is_empty() {
            return Ok(self.snapshot());
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move { inner.apply(&accepted, &request, guard).await })
            .await
            .map_err(|_| SettingsError::Aborted)?
    }
}

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
    let Some(problem) = text::problem(reason, MAX_REASON_LEN, false) else {
        return Ok(());
    };
    Err(SettingsError::Invalid {
        problems: vec![Problem {
            key: "reason".to_owned(),
            reason: problem,
        }],
    })
}
