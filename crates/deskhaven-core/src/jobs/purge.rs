use std::future::Future;
use std::time::Duration;

use deskhaven_db::begin_write;
use sea_orm::sea_query::{LockBehavior, LockType, Order, Query};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter,
};

pub const PURGE_BATCH: u64 = 1000;
/// Longer than the longest sleep of SQLite's busy handler (100 ms), so a waiting writer retries
/// while the lock is free.
pub const PURGE_PAUSE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Purge {
    batch: u64,
    pause: Duration,
}

impl Default for Purge {
    fn default() -> Self {
        Self::new(PURGE_BATCH, PURGE_PAUSE)
    }
}

pub(crate) struct Doomed<E: EntityTrait> {
    pub key: E::Column,
    pub order: E::Column,
    pub cond: Condition,
}

/// What it writes commits or rolls back with that batch.
pub(crate) trait BatchHook {
    fn on_batch(
        &mut self,
        tx: &DatabaseTransaction,
        deleted: u64,
    ) -> impl Future<Output = Result<(), DbErr>> + Send;
}

pub(crate) struct Plain;

impl BatchHook for Plain {
    async fn on_batch(&mut self, _: &DatabaseTransaction, _: u64) -> Result<(), DbErr> {
        Ok(())
    }
}

impl Purge {
    pub fn new(batch: u64, pause: Duration) -> Self {
        Self {
            batch: batch.max(1),
            pause,
        }
    }

    /// Stops after a short batch (rows another writer holds on PostgreSQL wait for the next run).
    /// Cancel-safe: a dropped future loses at most its uncommitted batch.
    pub(crate) async fn run<E, H>(
        &self,
        db: &DatabaseConnection,
        doomed: &Doomed<E>,
        hook: &mut H,
    ) -> Result<u64, DbErr>
    where
        E: EntityTrait,
        H: BatchHook,
    {
        let mut total = 0;
        loop {
            let tx = begin_write(db).await?;
            let deleted = self.delete_batch(&tx, doomed).await?;
            if deleted > 0 {
                hook.on_batch(&tx, deleted).await?;
            }
            tx.commit().await?;
            total += deleted;
            if deleted < self.batch {
                return Ok(total);
            }
            self.rest().await;
        }
    }

    async fn delete_batch<E: EntityTrait>(
        &self,
        tx: &DatabaseTransaction,
        doomed: &Doomed<E>,
    ) -> Result<u64, DbErr> {
        let mut victims = Query::select();
        victims
            .column(doomed.key)
            .from(E::default())
            .cond_where(doomed.cond.clone())
            .order_by(doomed.order, Order::Asc)
            .limit(self.batch)
            .lock_with_behavior(LockType::Update, LockBehavior::SkipLocked);
        let result = E::delete_many()
            .filter(doomed.key.in_subquery(victims))
            .exec(tx)
            .await?;
        Ok(result.rows_affected)
    }

    async fn rest(&self) {
        if self.pause.is_zero() {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(self.pause).await;
        }
    }
}
