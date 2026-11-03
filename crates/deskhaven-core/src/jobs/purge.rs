//! [`Purge`]: deletes the rows matching a condition one batch per write transaction (see
//! the [module documentation](super)).

use std::future::Future;
use std::time::Duration;

use deskhaven_db::begin_write;
use sea_orm::sea_query::{LockBehavior, LockType, Order, Query};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter,
};

/// Rows deleted per write transaction.
pub const PURGE_BATCH: u64 = 1000;
/// Pause between two batches: longer than the longest sleep of SQLite's busy handler
/// (100 ms), so a writer waiting for the lock retries while it is free.
pub const PURGE_PAUSE: Duration = Duration::from_millis(150);

/// Batch size and pause of the batched deletes.
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

/// The rows to delete from `E`: those matching `cond`, oldest `order` first, by `key`.
pub(crate) struct Doomed<E: EntityTrait> {
    pub key: E::Column,
    pub order: E::Column,
    pub cond: Condition,
}

/// Runs inside each batch's transaction; what it stages is applied only once that
/// transaction committed.
pub(crate) trait BatchHook {
    type Staged: Send;

    /// `deleted` (> 0) rows are gone in `tx`, not yet committed.
    fn on_batch(
        &self,
        tx: &DatabaseTransaction,
        deleted: u64,
    ) -> impl Future<Output = Result<Self::Staged, DbErr>> + Send;

    /// The batch committed.
    fn committed(&mut self, staged: Self::Staged);
}

/// No hook.
pub(crate) struct Plain;

impl BatchHook for Plain {
    type Staged = ();

    async fn on_batch(&self, _: &DatabaseTransaction, _: u64) -> Result<(), DbErr> {
        Ok(())
    }

    fn committed(&mut self, (): ()) {}
}

impl Purge {
    /// `batch` rows (at least 1) per transaction, `pause` between two batches.
    pub fn new(batch: u64, pause: Duration) -> Self {
        Self {
            batch: batch.max(1),
            pause,
        }
    }

    /// Deletes every row of `doomed`, a batch per transaction; returns how many. Stops after
    /// a short batch (rows another writer holds on PostgreSQL wait for the next run).
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
            let staged = match deleted {
                0 => None,
                _ => Some(hook.on_batch(&tx, deleted).await?),
            };
            tx.commit().await?;
            if let Some(staged) = staged {
                hook.committed(staged);
            }
            total += deleted;
            if deleted < self.batch {
                return Ok(total);
            }
            self.rest().await;
        }
    }

    /// `DELETE … WHERE key IN (SELECT key … ORDER BY order LIMIT batch FOR UPDATE SKIP
    /// LOCKED)` (SQLite has a single writer and no row locks; the clause is not emitted).
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

    /// Lets other writers in between two batches.
    async fn rest(&self) {
        if self.pause.is_zero() {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(self.pause).await;
        }
    }
}
