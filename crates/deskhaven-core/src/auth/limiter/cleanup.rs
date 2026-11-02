//! Maintenance of `auth_lockouts` (scheduled by M2-20): retention and a hard row cap.
//!
//! Every delete selects its victims with `FOR UPDATE SKIP LOCKED` (PostgreSQL; SQLite
//! serializes writers and ignores the clause): a row that a login is working on right now
//! is left for the next run, and cleanup never waits for a row lock, so it cannot take part
//! in a deadlock with the global-first lock order of the login paths.

use std::time::Duration;

use deskhaven_db::entity::auth_lockout::{self, Column};
use sea_orm::sea_query::{LockBehavior, LockType, Order, Query};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait, PaginatorTrait, QueryFilter};

use super::lockout::{AccountLockout, KNOWN_SOURCE_RETENTION, at};
use super::policy::{LEVEL_DECAY, MAX_LOCK, millis};

/// A row older than this no longer affects any decision: a window lasts at most 24h (the
/// `auth.lockout.window` maximum, equal to [`MAX_LOCK`]), a lock set at `updated_at` ends by
/// `updated_at + MAX_LOCK`, and its level is forgotten [`LEVEL_DECAY`] after that.
pub const LOCKOUT_RETENTION: Duration = MAX_LOCK.saturating_add(LEVEL_DECAY);
/// A row that never locked only matters while its window is open (at most 24h).
pub const UNLOCKED_RETENTION: Duration = MAX_LOCK;

impl AccountLockout {
    /// Deletes rows that no longer affect any decision, then enforces the row cap; returns
    /// how many rows were deleted.
    ///
    /// 1. Retention: untouched for [`LOCKOUT_RETENTION`], or never locked and untouched for
    ///    [`UNLOCKED_RETENTION`] — unless it is a known source (a success within
    ///    [`KNOWN_SOURCE_RETENTION`]).
    /// 2. Cap: while more than the cap ([`AccountLockout::with_max_rows`]) remain, the oldest
    ///    rows that are neither locked nor known sources go first. Locked and known rows are
    ///    never evicted, so the cap can be exceeded by them alone.
    pub async fn cleanup(&self) -> Result<u64, DbErr> {
        let now = self.clock.now_ms();
        let cutoff = |age: Duration| at(now.saturating_sub(millis(age)));
        let unknown = Condition::any()
            .add(Column::LastSuccessAt.is_null())
            .add(Column::LastSuccessAt.lte(cutoff(KNOWN_SOURCE_RETENTION)?));
        let expired = Condition::any()
            .add(Column::UpdatedAt.lte(cutoff(LOCKOUT_RETENTION)?))
            .add(
                Condition::all()
                    .add(Column::LockedUntil.is_null())
                    .add(Column::UpdatedAt.lte(cutoff(UNLOCKED_RETENTION)?)),
            );
        let mut deleted = self
            .delete_skip_locked(Condition::all().add(expired).add(unknown), None)
            .await?;

        let rows = auth_lockout::Entity::find().count(&self.db).await?;
        let excess = rows.saturating_sub(self.max_rows);
        if excess > 0 {
            let evictable = Condition::all()
                .add(Column::LockedUntil.is_null())
                .add(Column::LastSuccessAt.is_null());
            deleted += self.delete_skip_locked(evictable, Some(excess)).await?;
            tracing::warn!(
                rows,
                cap = self.max_rows,
                "auth_lockouts over its row cap; evicted oldest counters"
            );
        }
        Ok(deleted)
    }

    /// `DELETE … WHERE key IN (SELECT key … [ORDER BY updated_at LIMIT n] FOR UPDATE SKIP
    /// LOCKED)`.
    async fn delete_skip_locked(&self, cond: Condition, oldest: Option<u64>) -> Result<u64, DbErr> {
        let mut victims = Query::select();
        victims
            .column(Column::Key)
            .from(auth_lockout::Entity)
            .cond_where(cond)
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
        if let Some(n) = oldest {
            victims.order_by(Column::UpdatedAt, Order::Asc).limit(n);
        }
        let result = auth_lockout::Entity::delete_many()
            .filter(Column::Key.in_subquery(victims))
            .exec(&self.db)
            .await?;
        Ok(result.rows_affected)
    }
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
