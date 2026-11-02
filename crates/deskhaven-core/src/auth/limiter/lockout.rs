//! Persistent attempt counting and lockout (`auth_lockouts`), count-before-verify.
//!
//! Keys (`key.rs`) never depend on whether the account exists, so an unknown username is
//! counted and locked exactly like a real one. The state machine lives in `policy.rs`.
//!
//! # Concurrency
//!
//! [`AccountLockout::reserve`] is one short [`begin_write`] transaction over both rows of the
//! attempt: `INSERT … ON CONFLICT (key) DO UPDATE SET key = excluded.key RETURNING *` (row
//! created if missing, returned, and on PostgreSQL row-locked until commit), then the next
//! states are written. SQLite: `BEGIN IMMEDIATE` makes concurrent writers wait their turn.
//! PostgreSQL: the upsert's row lock serializes the read-modify-write; a concurrent success
//! `DELETE` waits for it instead of making the row vanish mid-transaction. Either way no
//! attempt is lost, and every lock cycle grants at most `threshold` attempts.
//!
//! **Lock order** (PostgreSQL deadlock freedom): every transaction touching a username's
//! rows locks its global row (`user:<name>|*`) before any of its source rows, one statement
//! per row. `reserve`, `release`, `refund` and `unlock` all follow it; `cleanup` only
//! deletes rows nobody has touched for a day or more.

use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::auth_lockout::{self, Column};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{Set, Unchanged};
use sea_orm::sea_query::{Expr, LikeExpr, OnConflict, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection,
    DatabaseTransaction, DbErr, EntityTrait, QueryFilter, QuerySelect,
};

use super::key::{LIKE_ESCAPE, Subject, global_key_of, name_of, username_pattern};
use super::policy::{LEVEL_DECAY, MAX_LOCK, Policy, Reservation, State, millis};
use crate::clock::SharedClock;
use crate::settings::SettingsService;

/// A row older than this no longer affects any decision: a window lasts at most 24h (the
/// `auth.lockout.window` maximum, equal to [`MAX_LOCK`]), a lock set at `updated_at` ends by
/// `updated_at + MAX_LOCK`, and its level is forgotten [`LEVEL_DECAY`] after that.
pub const LOCKOUT_RETENTION: Duration = MAX_LOCK.saturating_add(LEVEL_DECAY);
/// A row that never locked only matters while its window is open (at most 24h).
pub const UNLOCKED_RETENTION: Duration = MAX_LOCK;

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
    db: DatabaseConnection,
    settings: SettingsService,
    clock: SharedClock,
}

impl AccountLockout {
    pub fn new(db: DatabaseConnection, settings: SettingsService, clock: SharedClock) -> Self {
        Self {
            db,
            settings,
            clock,
        }
    }

    /// Whether `subject` is locked now (by its source row or the global row). Read-only.
    pub async fn check(&self, subject: &Subject) -> Result<LockStatus, DbErr> {
        let now = self.clock.now_ms();
        let rows = auth_lockout::Entity::find()
            .filter(Column::Key.is_in([subject.source_key(), subject.global_key()]))
            .all(&self.db)
            .await?;
        let retry_after = rows
            .iter()
            .filter_map(|row| state_of(row).retry_after(now))
            .max();
        Ok(LockStatus::of(retry_after))
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
        // Lock order: global row first (see module docs).
        let global = state_of(&upsert(&txn, &global_key, now_ms).await?);
        let source = state_of(&upsert(&txn, &source_key, now_ms).await?);
        let locked = [source, global]
            .iter()
            .filter_map(|s| s.retry_after(now_ms))
            .max();
        if let Some(retry_after) = locked {
            txn.rollback().await?;
            return Ok(Reserved::Locked { retry_after });
        }
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
        let retry_after = [next_source, next_global]
            .iter()
            .filter_map(|s| s.retry_after(now_ms))
            .max();
        Ok(Reserved::Granted(Attempt {
            subject: subject.clone(),
            source,
            global,
            status: LockStatus::of(retry_after),
        }))
    }

    /// The attempt succeeded: forgets the counters (and backoff) of its source row and the
    /// global row. Other sources' rows stay.
    pub async fn release(&self, attempt: Attempt) -> Result<(), DbErr> {
        let txn = begin_write(&self.db).await?;
        // One statement per row, global first: `key IN (..)` would lock in heap order.
        for key in [attempt.subject.global_key(), attempt.subject.source_key()] {
            auth_lockout::Entity::delete_by_id(key).exec(&txn).await?;
        }
        txn.commit().await
    }

    /// Verification never ran (e.g. the password hasher was busy): gives the attempt back.
    pub async fn refund(&self, attempt: Attempt) -> Result<(), DbErr> {
        let now = self.clock.now();
        let txn = begin_write(&self.db).await?;
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

    /// Clears every row of `username` (global and all sources); returns how many. For the
    /// admin unlock (M3-04).
    pub async fn unlock(&self, username: &str) -> Result<u64, DbErr> {
        let name = name_of(username);
        let txn = begin_write(&self.db).await?;
        // Global row first (lock order), then the rest in scan order: every other writer
        // already holds or waits for the global row, so none can hold a source row we need
        // while waiting for us.
        auth_lockout::Entity::find_by_id(global_key_of(&name))
            .lock_exclusive()
            .one(&txn)
            .await?;
        let pattern = LikeExpr::new(username_pattern(&name)).escape(LIKE_ESCAPE);
        let result = auth_lockout::Entity::delete_many()
            .filter(Column::Key.like(pattern))
            .exec(&txn)
            .await?;
        txn.commit().await?;
        Ok(result.rows_affected)
    }

    /// Deletes rows that no longer affect any decision: untouched for
    /// [`LOCKOUT_RETENTION`], or never locked and untouched for [`UNLOCKED_RETENTION`].
    /// Returns how many. Scheduled by the maintenance task (M2-20).
    pub async fn cleanup(&self) -> Result<u64, DbErr> {
        let now = self.clock.now_ms();
        let cutoff = |age: Duration| at(now.saturating_sub(millis(age)));
        let stale = Condition::any()
            .add(Column::UpdatedAt.lte(cutoff(LOCKOUT_RETENTION)?))
            .add(
                Condition::all()
                    .add(Column::LockedUntil.is_null())
                    .add(Column::UpdatedAt.lte(cutoff(UNLOCKED_RETENTION)?)),
            );
        let result = auth_lockout::Entity::delete_many()
            .filter(stale)
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
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

fn at(ms: i64) -> Result<TimestampMs, DbErr> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_covers_window_lock_and_decay() {
        assert_eq!(LOCKOUT_RETENTION, Duration::from_secs(48 * 60 * 60));
        assert_eq!(UNLOCKED_RETENTION, Duration::from_secs(24 * 60 * 60));
    }
}
