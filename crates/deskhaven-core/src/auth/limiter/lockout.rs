//! Persistent attempt counting and lockout (`auth_lockouts`), count-before-verify.
//!
//! Keys (`key.rs`) never depend on whether the account exists, so an unknown username is
//! counted and locked exactly like a real one. The state machine lives in `policy.rs`,
//! maintenance (retention, row cap) in `cleanup.rs`.
//!
//! # Known sources
//!
//! A successful login keeps its source row (count, lock and level cleared) and stamps
//! `last_success_at`. For [`KNOWN_SOURCE_RETENTION`] after that, the source is **known**:
//! the global row does not block it (its own row still does). This keeps a distributed
//! attack, which can hold the global row locked indefinitely, from locking out a user who
//! logs in from where they logged in before. Fresh sources (zero failures included) get no
//! such pass, otherwise the global row would bind nobody.
//!
//! # Concurrency
//!
//! [`AccountLockout::reserve`] is one short [`begin_write`] transaction over both rows of the
//! attempt: `INSERT … ON CONFLICT (key) DO UPDATE SET key = excluded.key RETURNING *` (row
//! created if missing, returned, and on PostgreSQL row-locked until commit), then the next
//! states are written. SQLite: `BEGIN IMMEDIATE` makes concurrent writers wait their turn.
//! PostgreSQL: the upsert's row lock serializes the read-modify-write; a concurrent success
//! waits for it instead of changing the row mid-transaction. Either way no attempt is lost,
//! and every lock cycle grants at most `threshold` attempts.
//!
//! **Deadlock freedom** (PostgreSQL): every write transaction on a username's rows
//! (`reserve`, `release`, `complete`, `refund`, `unlock`) first takes a transaction-scoped
//! advisory lock on that username, so they run one after another and never hold row locks
//! the others wait for. Ordering row locks alone is not enough: a row that does not exist
//! yet (a deleted global row) cannot be locked first. Each transaction touches a single
//! username, so there is no cross-username cycle either. `cleanup` never waits for a row
//! lock at all (`FOR UPDATE SKIP LOCKED`).

use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::auth_lockout::{self, Column};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{NotSet, Set, Unchanged};
use sea_orm::sea_query::{Expr, LikeExpr, OnConflict, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction,
    DbBackend, DbErr, EntityTrait, QueryFilter, QuerySelect, Statement,
};

use super::key::{LIKE_ESCAPE, Subject, global_key_of, name_of, username_pattern};
use super::policy::{Policy, Reservation, State, millis};
use crate::clock::SharedClock;
use crate::settings::SettingsService;

/// How long a successful login makes its source a known source of the username.
pub const KNOWN_SOURCE_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// Default hard cap on `auth_lockouts` rows ([`AccountLockout::with_max_rows`]).
pub const MAX_LOCKOUT_ROWS: u64 = 100_000;

/// Outcome of a read-only lockout check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStatus {
    Allowed,
    Locked { retry_after: Duration },
}

impl LockStatus {
    fn of(retry_after: Option<Duration>) -> Self {
        match retry_after {
            Some(retry_after) => Self::Locked { retry_after },
            None => Self::Allowed,
        }
    }
}

/// One counted login attempt, granted by [`AccountLockout::reserve`]. Report its outcome
/// with [`AccountLockout::release`] (success) or [`AccountLockout::refund`] (verification
/// never ran); a failure needs nothing more, the attempt is already counted. Dropping it
/// counts as a failure.
#[derive(Debug)]
#[must_use = "report the outcome: success releases, a verification that never ran refunds"]
pub struct Attempt {
    subject: Subject,
    source: Reservation,
    global: Reservation,
    status: LockStatus,
}

impl Attempt {
    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    /// `Locked` when this attempt reached a threshold and started a lock (it is still
    /// verified; the next one is refused). For login events.
    pub fn status(&self) -> LockStatus {
        self.status
    }
}

/// The first step of a two-step login passed; the attempt stays counted until the second
/// factor completes ([`AccountLockout::complete`]). Store [`PendingSubject::parts`] with the
/// challenge and rebuild it with [`PendingSubject::from_parts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSubject(Subject);

impl PendingSubject {
    pub(super) fn from_subject(subject: Subject) -> Self {
        Self(subject)
    }

    pub fn subject(&self) -> &Subject {
        &self.0
    }

    /// `(name, source)`, both plain text, for storage.
    pub fn parts(&self) -> (&str, &str) {
        (self.0.name(), self.0.source())
    }

    /// `None` unless the parts are exactly what [`PendingSubject::parts`] produces.
    pub fn from_parts(name: &str, source: &str) -> Option<Self> {
        Subject::from_parts(name, source).map(Self)
    }
}

/// Outcome of [`AccountLockout::reserve`].
#[derive(Debug)]
pub enum Reserved {
    Granted(Attempt),
    Locked { retry_after: Duration },
}

/// Per-username attempt counters with lockout. Settings are read on every call, so changes
/// apply immediately. Cheap to clone.
#[derive(Clone)]
pub struct AccountLockout {
    pub(super) db: DatabaseConnection,
    pub(super) settings: SettingsService,
    pub(super) clock: SharedClock,
    pub(super) max_rows: u64,
}

impl AccountLockout {
    pub fn new(db: DatabaseConnection, settings: SettingsService, clock: SharedClock) -> Self {
        Self {
            db,
            settings,
            clock,
            max_rows: MAX_LOCKOUT_ROWS,
        }
    }

    /// The same lockout with another hard row cap (see `cleanup`).
    pub fn with_max_rows(self, max_rows: u64) -> Self {
        Self { max_rows, ..self }
    }

    /// Whether `subject` is locked now (by its source row, or by the global row unless it is
    /// a known source). Read-only.
    pub async fn check(&self, subject: &Subject) -> Result<LockStatus, DbErr> {
        let now = self.clock.now_ms();
        let find = |key: String| auth_lockout::Entity::find_by_id(key).one(&self.db);
        let global = find(subject.global_key()).await?;
        let source = find(subject.source_key()).await?;
        Ok(LockStatus::of(blocking(
            source.as_ref(),
            global.as_ref(),
            now,
        )))
    }

    /// Counts one attempt for `subject` unless it is locked. The attempt that reaches a
    /// threshold is granted (and starts the lock); the next one is refused.
    pub async fn reserve(&self, subject: &Subject) -> Result<Reserved, DbErr> {
        let snapshot = self.settings.snapshot();
        let (source_policy, global_policy) =
            (Policy::per_source(&snapshot), Policy::global(&snapshot));
        let (source_key, global_key) = (subject.source_key(), subject.global_key());
        let now = self.clock.now();
        let now_ms = now.timestamp_millis();

        let txn = begin_write(&self.db).await?;
        lock_username(&txn, subject.name()).await?;
        let global_row = upsert(&txn, &global_key, now_ms).await?;
        let source_row = upsert(&txn, &source_key, now_ms).await?;
        if let Some(retry_after) = blocking(Some(&source_row), Some(&global_row), now_ms) {
            txn.rollback().await?;
            return Ok(Reserved::Locked { retry_after });
        }
        let (global, source) = (state_of(&global_row), state_of(&source_row));
        let next_source = source_policy.on_attempt(source, now_ms);
        let next_global = global_policy.on_attempt(global, now_ms);
        write(&txn, &global_key, next_global, now).await?;
        write(&txn, &source_key, next_source, now).await?;
        txn.commit().await?;

        let source = Reservation {
            before: source,
            after: next_source,
        };
        let global = Reservation {
            before: global,
            after: next_global,
        };
        warn_new_lock(subject, "source", &source, now_ms);
        warn_new_lock(subject, "global", &global, now_ms);
        // Only locks this attempt started (a known source may pass a global lock in force).
        let retry_after = [source, global]
            .iter()
            .filter(|r| r.before.starts_lock(&r.after))
            .filter_map(|r| r.after.retry_after(now_ms))
            .max();
        Ok(Reserved::Granted(Attempt {
            subject: subject.clone(),
            source,
            global,
            status: LockStatus::of(retry_after),
        }))
    }

    /// The attempt completed a login: deletes the username's global row and makes the
    /// attempt's source a known source (count, lock and level cleared, `last_success_at`
    /// stamped). Other sources' rows stay.
    pub async fn release(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.mark_known(attempt.subject.name(), &[attempt.subject.source_key()])
            .await
    }

    /// A two-step login completed: releases the first step's source and, when it is the same
    /// username, the second factor's source too.
    pub async fn complete(&self, pending: PendingSubject, second: Attempt) -> Result<(), DbErr> {
        let first = pending.0;
        let mut sources = vec![first.source_key()];
        if second.subject.name() == first.name() && second.subject != first {
            sources.push(second.subject.source_key());
        }
        self.mark_known(first.name(), &sources).await
    }

    /// Verification never ran (e.g. the password hasher was busy): gives the attempt back.
    pub async fn refund(&self, attempt: Attempt) -> Result<(), DbErr> {
        let now = self.clock.now();
        let txn = begin_write(&self.db).await?;
        lock_username(&txn, attempt.subject.name()).await?;
        let rows = [
            (attempt.subject.global_key(), attempt.global),
            (attempt.subject.source_key(), attempt.source),
        ];
        for (key, reservation) in rows {
            let row = auth_lockout::Entity::find_by_id(key.as_str())
                .lock_exclusive()
                .one(&txn)
                .await?;
            let Some(row) = row else { continue };
            let current = state_of(&row);
            if let Some(next) = reservation.refund(current).filter(|next| *next != current) {
                write(&txn, &key, next, now).await?;
            }
        }
        txn.commit().await
    }

    /// Clears every row of `username` (global and all sources, known ones included);
    /// returns how many. For the admin unlock (M3-04).
    pub async fn unlock(&self, username: &str) -> Result<u64, DbErr> {
        let name = name_of(username);
        let txn = begin_write(&self.db).await?;
        lock_username(&txn, &name).await?;
        let pattern = LikeExpr::new(username_pattern(&name)).escape(LIKE_ESCAPE);
        let result = auth_lockout::Entity::delete_many()
            .filter(Column::Key.like(pattern))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(result.rows_affected)
    }

    /// Deletes `name`'s global row, then resets each source row to a known source.
    async fn mark_known(&self, name: &str, source_keys: &[String]) -> Result<(), DbErr> {
        let now = self.clock.now();
        let now_ms = now.timestamp_millis();
        let txn = begin_write(&self.db).await?;
        lock_username(&txn, name).await?;
        auth_lockout::Entity::delete_by_id(global_key_of(name))
            .exec(&txn)
            .await?;
        for key in source_keys {
            upsert(&txn, key, now_ms).await?;
            auth_lockout::ActiveModel {
                key: Unchanged(key.clone()),
                failures: Set(0),
                window_started_at: Set(now),
                locked_until: Set(None),
                lock_level: Set(0),
                updated_at: Set(now),
                last_success_at: Set(Some(now)),
            }
            .update(&txn)
            .await?;
        }
        txn.commit().await
    }
}

/// PostgreSQL advisory lock namespace of the per-username lock (ASCII "dhlk"). The two-key
/// form lives apart from single-`bigint` keys such as the migration lock.
const PG_USERNAME_LOCK_NS: i32 = 0x6468_6c6b;

/// Serializes the write transactions of one username (see module docs). PostgreSQL: a
/// transaction-scoped advisory lock, released at commit or rollback. SQLite: nothing to do,
/// `BEGIN IMMEDIATE` already serializes all writers. A hash collision only makes two
/// usernames wait for each other.
async fn lock_username(txn: &DatabaseTransaction, name: &str) -> Result<(), DbErr> {
    if txn.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    let sql = "SELECT pg_advisory_xact_lock($1, hashtext($2))";
    let values = [PG_USERNAME_LOCK_NS.into(), name.into()];
    txn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        sql,
        values,
    ))
    .await?;
    Ok(())
}

/// How long the source and global rows block an attempt at `now`; the global row does not
/// block a known source.
fn blocking(
    source: Option<&auth_lockout::Model>,
    global: Option<&auth_lockout::Model>,
    now: i64,
) -> Option<Duration> {
    let known = source.is_some_and(|row| is_known(row, now));
    let global = global.filter(|_| !known);
    [source, global]
        .into_iter()
        .flatten()
        .filter_map(|row| state_of(row).retry_after(now))
        .max()
}

pub(super) fn is_known(row: &auth_lockout::Model, now: i64) -> bool {
    row.last_success_at.is_some_and(|at| {
        now.saturating_sub(at.timestamp_millis()) < millis(KNOWN_SOURCE_RETENTION)
    })
}

/// Returns the row of `key`, inserting a fresh one if missing; on PostgreSQL the row stays
/// locked until the transaction ends (the no-op `DO UPDATE` takes the row lock).
async fn upsert(
    txn: &DatabaseTransaction,
    key: &str,
    now_ms: i64,
) -> Result<auth_lockout::Model, DbErr> {
    let fresh = State::fresh(now_ms);
    let insert = Query::insert()
        .into_table(auth_lockout::Entity)
        .columns([
            Column::Key,
            Column::Failures,
            Column::WindowStartedAt,
            Column::LockedUntil,
            Column::LockLevel,
            Column::UpdatedAt,
        ])
        .values([
            Expr::val(key),
            Expr::val(fresh.failures),
            Expr::val(fresh.window_started_at),
            Expr::val(fresh.locked_until),
            Expr::val(fresh.lock_level),
            Expr::val(now_ms),
        ])
        .map_err(|e| DbErr::Custom(format!("lockout upsert: {e}")))?
        .on_conflict(
            OnConflict::column(Column::Key)
                .update_column(Column::Key)
                .to_owned(),
        )
        .returning_all()
        .to_owned();
    let statement = txn.get_database_backend().build(&insert);
    auth_lockout::Entity::find()
        .from_raw_sql(statement)
        .one(txn)
        .await?
        .ok_or_else(|| DbErr::RecordNotFound(key.to_owned()))
}

/// Writes the counter columns; `last_success_at` is left as it is.
async fn write(
    txn: &DatabaseTransaction,
    key: &str,
    state: State,
    now: TimestampMs,
) -> Result<(), DbErr> {
    auth_lockout::ActiveModel {
        key: Unchanged(key.to_owned()),
        failures: Set(state.failures),
        window_started_at: Set(at(state.window_started_at)?),
        locked_until: Set(state.locked_until.map(at).transpose()?),
        lock_level: Set(state.lock_level),
        updated_at: Set(now),
        last_success_at: NotSet,
    }
    .update(txn)
    .await?;
    Ok(())
}

fn state_of(row: &auth_lockout::Model) -> State {
    State {
        failures: row.failures,
        window_started_at: row.window_started_at.timestamp_millis(),
        locked_until: row.locked_until.map(|t| t.timestamp_millis()),
        lock_level: row.lock_level,
    }
}

pub(super) fn at(ms: i64) -> Result<TimestampMs, DbErr> {
    time::from_millis(ms).ok_or_else(|| DbErr::Custom("lockout time out of range".to_owned()))
}

/// Never logs the password or anything derived from it.
fn warn_new_lock(subject: &Subject, scope: &'static str, r: &Reservation, now_ms: i64) {
    if !r.before.starts_lock(&r.after) {
        return;
    }
    let duration_s = r.after.retry_after(now_ms).map_or(0, |d| d.as_secs());
    tracing::warn!(
        username = subject.name(),
        source = subject.source(),
        scope,
        level = r.after.lock_level,
        duration_s,
        "login lockout started"
    );
}
