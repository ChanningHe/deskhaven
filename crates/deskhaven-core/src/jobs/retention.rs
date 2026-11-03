//! [`AuditRetention`]: audit records older than their `audit.retention.*` setting are
//! deleted, by `created_at` (indexed); a record created exactly at the cutoff is kept.
//! `connection_sessions` rows retired by the audit ingest (session id
//! [`RETIRED_PREFIX`](crate::audit::RETIRED_PREFIX)…) are ordinary rows here.
//!
//! **Its own audit trail** (06 §7), append-only: every batch that deletes anything inserts,
//! in its own transaction, one `console_audit` row (actor `system`, action
//! [`RETENTION_ACTION`], target `audit` / the run's id) with the type, the rows deleted, the
//! cutoff and the batch's number within the run. Whatever has been deleted, however the run
//! ends, is committed together with the row that records it; nothing is ever updated. A
//! run that deletes nothing writes nothing.

use std::time::Duration;

use deskhaven_config::settings::{
    AUDIT_RETENTION_ALARM, AUDIT_RETENTION_CONNECTION, AUDIT_RETENTION_CONSOLE,
    AUDIT_RETENTION_FILE, AUDIT_RETENTION_LOGIN, Setting,
};
use deskhaven_db::entity::{alarm, connection_session, console_audit, file_transfer, login_event};
use deskhaven_db::time::TimestampMs;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
};
use serde_json::json;

use super::purge::{BatchHook, Doomed, Purge};
use super::{chain, cutoff};
use crate::clock::SharedClock;
use crate::console_audit::{self as audit_log, Actor, NewConsoleAudit};
use crate::settings::SettingsService;

/// `console_audit.action` of a retention batch.
pub const RETENTION_ACTION: &str = "audit.retention.purge";
/// `console_audit.target_type` of a retention batch (`target_id`: the run's id).
pub const RETENTION_TARGET: &str = "audit";

/// The audit record types with a retention setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditKind {
    Connection,
    File,
    Alarm,
    Login,
    Console,
}

impl AuditKind {
    pub const ALL: [Self; 5] = [
        Self::Connection,
        Self::File,
        Self::Alarm,
        Self::Login,
        Self::Console,
    ];

    /// The name in `audit.retention.<name>` and in the audit rows.
    pub fn name(self) -> &'static str {
        match self {
            Self::Connection => "connection",
            Self::File => "file",
            Self::Alarm => "alarm",
            Self::Login => "login",
            Self::Console => "console",
        }
    }

    fn setting(self) -> &'static Setting<deskhaven_config::Duration> {
        match self {
            Self::Connection => &AUDIT_RETENTION_CONNECTION,
            Self::File => &AUDIT_RETENTION_FILE,
            Self::Alarm => &AUDIT_RETENTION_ALARM,
            Self::Login => &AUDIT_RETENTION_LOGIN,
            Self::Console => &AUDIT_RETENTION_CONSOLE,
        }
    }
}

/// What one [`AuditRetention::run`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RetentionReport {
    /// The run's id (`console_audit.target_id` of its rows).
    pub run_id: String,
    /// Records deleted, per type purged without error; a type kept forever is absent.
    pub deleted: Vec<(AuditKind, u64)>,
    /// Types whose purge failed (logged; retried by the next run).
    pub failed: Vec<AuditKind>,
}

impl RetentionReport {
    pub fn total(&self) -> u64 {
        self.deleted.iter().map(|(_, n)| n).sum()
    }

    pub fn deleted(&self, kind: AuditKind) -> u64 {
        self.deleted
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(0, |(_, n)| *n)
    }
}

/// The audit retention job.
#[derive(Clone)]
pub struct AuditRetention {
    db: DatabaseConnection,
    settings: SettingsService,
    clock: SharedClock,
    purge: Purge,
}

impl AuditRetention {
    pub fn new(db: DatabaseConnection, settings: SettingsService, clock: SharedClock) -> Self {
        Self {
            db,
            settings,
            clock,
            purge: Purge::default(),
        }
    }

    pub fn with_purge(self, purge: Purge) -> Self {
        Self { purge, ..self }
    }

    /// Deletes the expired records of every type (see the module documentation); a type
    /// that fails does not stop the others.
    pub async fn run(&self) -> RetentionReport {
        let now_ms = self.clock.now_ms();
        let mut report = RetentionReport {
            run_id: uuid::Uuid::now_v7().to_string(),
            ..RetentionReport::default()
        };
        let mut hook = AuditBatch {
            run_id: report.run_id.clone(),
            now_ms,
            kind: AuditKind::Connection,
            before: 0,
            batches: 0,
        };
        for (kind, before) in self.cutoffs(now_ms, &mut report.failed) {
            hook.kind = kind;
            hook.before = before.timestamp_millis();
            match self.purge_kind(&mut hook, before).await {
                Ok(deleted) => report.deleted.push((kind, deleted)),
                Err(err) => {
                    tracing::warn!(kind = kind.name(), error = %chain(&err), "audit retention failed; retrying next run");
                    report.failed.push(kind);
                }
            }
        }
        report
    }

    /// The cutoff of every type not kept forever, from one reading of the settings.
    fn cutoffs(&self, now_ms: i64, failed: &mut Vec<AuditKind>) -> Vec<(AuditKind, TimestampMs)> {
        let snapshot = self.settings.snapshot();
        let mut cutoffs = Vec::new();
        for kind in AuditKind::ALL {
            let retention: Duration = snapshot.get(kind.setting()).as_std();
            if retention.is_zero() {
                continue;
            }
            match cutoff(now_ms, retention) {
                Ok(before) => cutoffs.push((kind, before)),
                Err(err) => {
                    tracing::warn!(kind = kind.name(), error = %err, "audit retention cutoff out of range");
                    failed.push(kind);
                }
            }
        }
        cutoffs
    }

    async fn purge_kind(&self, hook: &mut AuditBatch, before: TimestampMs) -> Result<u64, DbErr> {
        let (db, purge) = (&self.db, &self.purge);
        let deleted = match hook.kind {
            AuditKind::Connection => {
                use connection_session::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), hook)
                    .await
            }
            AuditKind::File => {
                use file_transfer::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), hook)
                    .await
            }
            AuditKind::Alarm => {
                use alarm::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), hook)
                    .await
            }
            AuditKind::Login => {
                use login_event::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), hook)
                    .await
            }
            AuditKind::Console => {
                use console_audit::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), hook)
                    .await
            }
        }?;
        if deleted > 0 {
            tracing::info!(
                kind = hook.kind.name(),
                deleted,
                "expired audit records deleted"
            );
        }
        Ok(deleted)
    }
}

/// Rows created before `before`.
fn older<C: ColumnTrait>(key: C, created_at: C, before: TimestampMs) -> Doomed<C::EntityName>
where
    C::EntityName: EntityTrait<Column = C>,
{
    Doomed {
        key,
        order: created_at,
        cond: Condition::all().add(created_at.lt(before)),
    }
}

/// Inserts the audit row of each deleting batch, in its transaction.
struct AuditBatch {
    run_id: String,
    now_ms: i64,
    kind: AuditKind,
    /// The current type's cutoff (unix ms).
    before: i64,
    /// Batches recorded so far in this run (a rolled-back one leaves a gap).
    batches: u64,
}

impl BatchHook for AuditBatch {
    async fn on_batch(&mut self, tx: &DatabaseTransaction, deleted: u64) -> Result<(), DbErr> {
        self.batches += 1;
        let entry = NewConsoleAudit {
            actor: Actor::System,
            action: RETENTION_ACTION.to_owned(),
            target_type: RETENTION_TARGET.to_owned(),
            target_id: self.run_id.clone(),
            before: None,
            after: Some(json!({
                "kind": self.kind.name(),
                "deleted": deleted,
                "cutoff": self.before,
                "batch": self.batches,
            })),
            ip: None,
        };
        audit_log::insert(tx, &entry, self.now_ms).await.map(|_| ())
    }
}
