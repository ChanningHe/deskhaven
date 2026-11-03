//! [`AuditRetention`]: audit records older than their `audit.retention.*` setting are
//! deleted, by `created_at` (indexed). `connection_sessions` rows retired by the audit
//! ingest (session id [`RETIRED_PREFIX`](crate::audit::RETIRED_PREFIX)…) are ordinary rows
//! here.
//!
//! **Its own audit row** (06 §7): a run that deletes anything records one `console_audit`
//! row (actor `system`, action [`RETENTION_ACTION`]) with the cutoffs and the counts. The
//! row is inserted in the transaction of the run's first deleting batch and updated in the
//! transaction of every later one, so whatever has been deleted, however the run ends, is
//! committed together with a row that counts it. A run that deletes nothing writes nothing.

use std::time::Duration;

use deskhaven_config::settings::{
    AUDIT_RETENTION_ALARM, AUDIT_RETENTION_CONNECTION, AUDIT_RETENTION_CONSOLE,
    AUDIT_RETENTION_FILE, AUDIT_RETENTION_LOGIN, Setting,
};
use deskhaven_db::entity::{alarm, connection_session, console_audit, file_transfer, login_event};
use deskhaven_db::time::TimestampMs;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter,
};
use serde_json::{Map, Value, json};

use super::purge::{BatchHook, Doomed, Purge};
use super::{chain, cutoff};
use crate::clock::SharedClock;
use crate::console_audit::{self as audit_log, Actor, NewConsoleAudit};
use crate::settings::SettingsService;

/// `console_audit.action` of a retention run.
pub const RETENTION_ACTION: &str = "audit.retention.purge";
/// `console_audit.target_type` of a retention run.
const TARGET_TYPE: &str = "audit";

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

    /// The name in `audit.retention.<name>` and in the run's audit row.
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
    /// Records deleted, per type; a type kept forever is absent.
    pub deleted: Vec<(AuditKind, u64)>,
    /// Types whose purge failed (logged; retried by the next run).
    pub failed: Vec<AuditKind>,
    /// The run's `console_audit` row, if it deleted anything.
    pub audit_id: Option<i64>,
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

    /// Deletes the expired records of every type (see the module documentation).
    pub async fn run(&self) -> RetentionReport {
        let mut run = self.plan();
        for (kind, before) in run.cutoffs.clone() {
            if let Err(err) = self.purge_kind(&mut run, kind, before).await {
                tracing::warn!(kind = kind.name(), error = %chain(&err), "audit retention failed; retrying next run");
                run.report.failed.push(kind);
            }
        }
        run.report
    }

    /// The cutoff of every type not kept forever, all from one reading of the clock and the
    /// settings.
    fn plan(&self) -> Summary {
        let now_ms = self.clock.now_ms();
        let snapshot = self.settings.snapshot();
        let mut run = Summary::new(now_ms);
        for kind in AuditKind::ALL {
            let retention: Duration = snapshot.get(kind.setting()).as_std();
            if retention.is_zero() {
                continue;
            }
            match cutoff(now_ms, retention) {
                Ok(before) => run.start(kind, before),
                Err(err) => {
                    tracing::warn!(kind = kind.name(), error = %err, "audit retention cutoff out of range");
                    run.report.failed.push(kind);
                }
            }
        }
        run
    }

    async fn purge_kind(
        &self,
        run: &mut Summary,
        kind: AuditKind,
        before: TimestampMs,
    ) -> Result<(), DbErr> {
        let mut hook = SummaryHook { run, kind };
        let (db, purge) = (&self.db, &self.purge);
        let deleted = match kind {
            AuditKind::Connection => {
                use connection_session::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), &mut hook)
                    .await
            }
            AuditKind::File => {
                use file_transfer::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), &mut hook)
                    .await
            }
            AuditKind::Alarm => {
                use alarm::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), &mut hook)
                    .await
            }
            AuditKind::Login => {
                use login_event::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), &mut hook)
                    .await
            }
            AuditKind::Console => {
                use console_audit::Column as C;
                purge
                    .run(db, &older(C::Id, C::CreatedAt, before), &mut hook)
                    .await
            }
        }?;
        if deleted > 0 {
            tracing::info!(kind = kind.name(), deleted, "expired audit records deleted");
        }
        Ok(())
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

/// One run: its cutoffs, its committed counts and its audit row.
struct Summary {
    now_ms: i64,
    cutoffs: Vec<(AuditKind, TimestampMs)>,
    report: RetentionReport,
}

impl Summary {
    fn new(now_ms: i64) -> Self {
        Self {
            now_ms,
            cutoffs: Vec::new(),
            report: RetentionReport::default(),
        }
    }

    fn start(&mut self, kind: AuditKind, before: TimestampMs) {
        self.cutoffs.push((kind, before));
        self.report.deleted.push((kind, 0));
    }

    /// The counts with `more` deleted records of `kind`.
    fn counts_with(&self, kind: AuditKind, more: u64) -> Vec<(AuditKind, u64)> {
        let mut counts = self.report.deleted.clone();
        if let Some((_, n)) = counts.iter_mut().find(|(k, _)| *k == kind) {
            *n += more;
        }
        counts
    }

    /// The audit row's `after`.
    fn snapshot(&self, counts: &[(AuditKind, u64)]) -> Value {
        let named = |pairs: Vec<(AuditKind, i64)>| -> Value {
            let map: Map<String, Value> = pairs
                .into_iter()
                .map(|(k, v)| (k.name().to_owned(), Value::from(v)))
                .collect();
            Value::Object(map)
        };
        let deleted = counts
            .iter()
            .map(|(k, n)| (*k, i64::try_from(*n).unwrap_or(i64::MAX)))
            .collect();
        let cutoffs = self
            .cutoffs
            .iter()
            .map(|(k, before)| (*k, before.timestamp_millis()))
            .collect();
        json!({ "deleted": named(deleted), "cutoff": named(cutoffs) })
    }
}

/// Writes the run's audit row in each batch's transaction.
struct SummaryHook<'a> {
    run: &'a mut Summary,
    kind: AuditKind,
}

/// A batch's counts and the run's audit row, once committed.
struct Staged {
    counts: Vec<(AuditKind, u64)>,
    audit_id: i64,
}

impl BatchHook for SummaryHook<'_> {
    type Staged = Staged;

    async fn on_batch(&self, tx: &DatabaseTransaction, deleted: u64) -> Result<Staged, DbErr> {
        let counts = self.run.counts_with(self.kind, deleted);
        let after = self.run.snapshot(&counts);
        let audit_id = match self.run.report.audit_id {
            Some(id) => update_row(tx, id, after, self.run.now_ms).await?,
            None => insert_row(tx, after, self.run.now_ms).await?,
        };
        Ok(Staged { counts, audit_id })
    }

    fn committed(&mut self, staged: Staged) {
        self.run.report.deleted = staged.counts;
        self.run.report.audit_id = Some(staged.audit_id);
    }
}

async fn insert_row(tx: &DatabaseTransaction, after: Value, now_ms: i64) -> Result<i64, DbErr> {
    let entry = NewConsoleAudit {
        actor: Actor::System,
        action: RETENTION_ACTION.to_owned(),
        target_type: TARGET_TYPE.to_owned(),
        target_id: String::new(),
        before: None,
        after: Some(after),
        ip: None,
    };
    audit_log::insert(tx, &entry, now_ms).await
}

/// Updates the run's row; a row that is gone (never expected) is recreated, so the batch
/// still commits with a row that counts it.
async fn update_row(
    tx: &DatabaseTransaction,
    id: i64,
    after: Value,
    now_ms: i64,
) -> Result<i64, DbErr> {
    let text = audit_log::redact(&after).to_string();
    let updated = console_audit::Entity::update_many()
        .col_expr(console_audit::Column::After, Expr::value(text))
        .filter(console_audit::Column::Id.eq(id))
        .exec(tx)
        .await?;
    if updated.rows_affected == 1 {
        return Ok(id);
    }
    insert_row(tx, after, now_ms).await
}
