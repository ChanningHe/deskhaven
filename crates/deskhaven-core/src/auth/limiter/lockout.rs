//! Persistent per-account failure counting and lockout (`auth_lockouts`).
//!
//! Keys never depend on whether the account exists, so an unknown username is counted and
//! locked exactly like a real one. The state machine lives in `policy.rs`.
//!
//! # Concurrency
//!
//! [`AccountLockout::record_failure`] is one short [`begin_write`] transaction: insert a
//! fresh row if missing (`ON CONFLICT DO NOTHING`), read it with `FOR UPDATE`, write the next
//! state. SQLite: `BEGIN IMMEDIATE` takes the database write lock up front, so concurrent
//! recorders run one after another (the `FOR UPDATE` is not emitted). PostgreSQL: a racing
//! insert waits on the primary key and then does nothing, and the row lock serializes the
//! read-modify-write. Either way no increment is lost.

use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::auth_lockout;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{Set, Unchanged};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QuerySelect,
};

use super::policy::{LEVEL_DECAY, MAX_LOCK, Policy, State, millis};
use crate::clock::SharedClock;
use crate::settings::SettingsService;
use crate::users::username_norm;

/// Shared key of every malformed username (`~` is outside the username charset).
const MALFORMED_USERNAME_KEY: &str = "user:~malformed";
/// A row older than this no longer affects any decision: a window lasts at most
/// [`MAX_LOCK`] (the `auth.lockout.window` maximum is the same 24h), a lock set at
/// `updated_at` ends by `updated_at + MAX_LOCK`, and its level is forgotten
/// [`LEVEL_DECAY`] after that.
pub const LOCKOUT_RETENTION: Duration = MAX_LOCK.saturating_add(LEVEL_DECAY);

/// The `auth_lockouts` key of a login identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LockoutKey(String);

impl LockoutKey {
    /// `user:<username_norm>`, normalized exactly like `users.username_norm`
    /// ([`username_norm`]). Input that is not a well-formed username cannot name an account;
    /// it all shares one key, so attacker-chosen strings neither grow the table nor reach it
    /// verbatim (PostgreSQL rejects NUL in text).
    pub fn username(raw: &str) -> Self {
        match username_norm(raw) {
            Some(norm) => Self(format!("user:{norm}")),
            None => Self(MALFORMED_USERNAME_KEY.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Outcome of a lockout check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStatus {
    Allowed,
    Locked { retry_after: Duration },
}

impl LockStatus {
    fn of(state: &State, now: i64) -> Self {
        match state.retry_after(now) {
            Some(retry_after) => Self::Locked { retry_after },
            None => Self::Allowed,
        }
    }
}

/// Per-account failure counter with exponential-backoff lockout. Settings are read on
/// every call, so changes apply immediately. Cheap to clone.
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

    /// Whether `key` may attempt a login now. Read-only.
    pub async fn check(&self, key: &LockoutKey) -> Result<LockStatus, DbErr> {
        let now = self.clock.now_ms();
        let row = auth_lockout::Entity::find_by_id(key.as_str())
            .one(&self.db)
            .await?;
        Ok(row.map_or(LockStatus::Allowed, |row| {
            LockStatus::of(&state_of(&row), now)
        }))
    }

    /// Counts one failed login for `key`; returns the resulting status (a lock may have
    /// just started).
    pub async fn record_failure(&self, key: &LockoutKey) -> Result<LockStatus, DbErr> {
        let policy = Policy::from_snapshot(&self.settings.snapshot());
        let now = self.clock.now();
        let now_ms = now.timestamp_millis();
        let txn = begin_write(&self.db).await?;
        auth_lockout::Entity::insert(fresh_row(key, now))
            .on_conflict_do_nothing_on([auth_lockout::Column::Key])
            .exec_without_returning(&txn)
            .await?;
        let row = auth_lockout::Entity::find_by_id(key.as_str())
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or_else(|| DbErr::RecordNotFound(key.as_str().to_owned()))?;
        let current = state_of(&row);
        let next = policy.on_failure(current, now_ms);
        if next != current {
            updated_row(key, next, now)?.update(&txn).await?;
        }
        txn.commit().await?;
        Ok(LockStatus::of(&next, now_ms))
    }

    /// A successful login: forgets failures and backoff level of `key`.
    pub async fn record_success(&self, key: &LockoutKey) -> Result<(), DbErr> {
        auth_lockout::Entity::delete_by_id(key.as_str())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Deletes rows that no longer affect any decision (untouched for
    /// [`LOCKOUT_RETENTION`]); returns how many. Scheduled by the maintenance task (M2-20).
    pub async fn cleanup(&self) -> Result<u64, DbErr> {
        let cutoff = self
            .clock
            .now_ms()
            .saturating_sub(millis(LOCKOUT_RETENTION));
        let cutoff = time::from_millis(cutoff)
            .ok_or_else(|| DbErr::Custom("lockout cleanup cutoff out of range".to_owned()))?;
        let result = auth_lockout::Entity::delete_many()
            .filter(auth_lockout::Column::UpdatedAt.lte(cutoff))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
}

fn state_of(row: &auth_lockout::Model) -> State {
    State {
        failures: row.failures,
        window_started_at: row.window_started_at.timestamp_millis(),
        locked_until: row.locked_until.map(|t| t.timestamp_millis()),
        lock_level: row.lock_level,
    }
}

fn fresh_row(key: &LockoutKey, now: TimestampMs) -> auth_lockout::ActiveModel {
    let state = State::fresh(now.timestamp_millis());
    auth_lockout::ActiveModel {
        key: Set(key.as_str().to_owned()),
        failures: Set(state.failures),
        window_started_at: Set(now),
        locked_until: Set(None),
        lock_level: Set(state.lock_level),
        updated_at: Set(now),
    }
}

fn updated_row(
    key: &LockoutKey,
    state: State,
    now: TimestampMs,
) -> Result<auth_lockout::ActiveModel, DbErr> {
    let at = |ms: i64| {
        time::from_millis(ms).ok_or_else(|| DbErr::Custom("lockout time out of range".to_owned()))
    };
    Ok(auth_lockout::ActiveModel {
        key: Unchanged(key.as_str().to_owned()),
        failures: Set(state.failures),
        window_started_at: Set(at(state.window_started_at)?),
        locked_until: Set(state.locked_until.map(at).transpose()?),
        lock_level: Set(state.lock_level),
        updated_at: Set(now),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_keys_are_case_insensitive() {
        assert_eq!(LockoutKey::username("Alice").as_str(), "user:alice");
        assert_eq!(LockoutKey::username("alice"), LockoutKey::username("ALICE"));
        assert_eq!(LockoutKey::username("a.b_c@d-e").as_str(), "user:a.b_c@d-e");
    }

    #[test]
    fn malformed_usernames_share_one_key() {
        let long = "a".repeat(65);
        for raw in [
            "",
            "bad name",
            "nul\0byte",
            "ümlaut",
            long.as_str(),
            "~malformed",
        ] {
            assert_eq!(
                LockoutKey::username(raw).as_str(),
                MALFORMED_USERNAME_KEY,
                "{raw:?}"
            );
        }
        let longest = "a".repeat(64);
        assert_ne!(
            LockoutKey::username(&longest).as_str(),
            MALFORMED_USERNAME_KEY
        );
    }

    #[test]
    fn retention_covers_window_lock_and_decay() {
        assert_eq!(LOCKOUT_RETENTION, Duration::from_secs(48 * 60 * 60));
    }
}
