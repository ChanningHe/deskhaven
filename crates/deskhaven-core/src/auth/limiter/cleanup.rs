//! Every delete uses `FOR UPDATE SKIP LOCKED` (PostgreSQL): a row a login is working on is left
//! for the next run, and cleanup never waits for a row lock, so it cannot deadlock with the
//! login paths.

use std::time::Duration;

use deskhaven_db::entity::auth_lockout::{self, Column};
use sea_orm::sea_query::{LockBehavior, LockType, Order, Query};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait, PaginatorTrait, QueryFilter};

use super::lockout::{AccountLockout, KNOWN_SOURCE_RETENTION, at};
use super::policy::{LEVEL_DECAY, MAX_LOCK, millis};

/// A window lasts at most [`MAX_LOCK`], a lock ends by `updated_at + MAX_LOCK`, and its level is
/// forgotten [`LEVEL_DECAY`] after that.
pub const LOCKOUT_RETENTION: Duration = MAX_LOCK.saturating_add(LEVEL_DECAY);
/// Only matters while its window is open.
pub const UNLOCKED_RETENTION: Duration = MAX_LOCK;

impl AccountLockout {
    /// Locked and known-source rows are never evicted, so they alone can exceed the cap.
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
