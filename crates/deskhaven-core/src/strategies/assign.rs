//! Assignments: one strategy per target, replace semantics.
//!
//! `strategy_assignments.target_id` has no foreign key (it points at four tables), so the
//! deletion paths of those tables call the `forget_*` functions in their own transaction.
//! They cannot invalidate the cache themselves (nothing is committed yet): the caller does
//! it after commit, through [`StrategyCache::invalidate`](super::StrategyCache::invalidate)
//! (users and groups: `StrategyService::watch_users` / `watch_groups`).

use deskhaven_db::begin_write;
use deskhaven_db::entity::strategy_assignment;
use deskhaven_db::entity::{device, device_group, group, strategy, user};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect,
    TryInsert, TryInsertResult,
};

use super::load;
use super::model::{Assignment, Target};
use super::{StrategyError, StrategyService};

impl StrategyService {
    /// Makes `strategy_id` the strategy of `target`, replacing the one it had. Assigning the
    /// strategy it already has is a no-op that still advances the version.
    pub async fn assign(&self, target: Target, strategy_id: i64) -> Result<(), StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            ensure_strategy(&tx, strategy_id).await?;
            ensure_target(&tx, target).await?;
            upsert(&tx, target, strategy_id).await?;
            tx.commit().await?;
            inner.cache.invalidate();
            Ok(())
        })
        .await
    }

    /// Removes the strategy of `target`; `false` if it had none.
    pub async fn unassign(&self, target: Target) -> Result<bool, StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let removed = forget_target(&tx, target).await? > 0;
            tx.commit().await?;
            if removed {
                inner.cache.invalidate();
            }
            Ok(removed)
        })
        .await
    }

    /// The strategy id assigned to `target`.
    pub async fn assignment(&self, target: Target) -> Result<Option<i64>, StrategyError> {
        let row = strategy_assignment::Entity::find()
            .filter(target_filter(target))
            .one(&self.inner.db)
            .await?;
        Ok(row.map(|r| r.strategy_id))
    }

    /// Every assignment, ordered by target type and id.
    pub async fn assignments(&self) -> Result<Vec<Assignment>, StrategyError> {
        let mut all = load::assignments(&self.inner.db).await?;
        all.sort_by_key(|a| a.target);
        Ok(all)
    }
}

/// Removes the assignment of a deleted user. Call in the transaction that deletes the row.
pub async fn forget_user(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::User(id)).await.map(drop)
}

/// Removes the assignment of a deleted user group. Call in the transaction that deletes it.
pub async fn forget_group(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::Group(id)).await.map(drop)
}

/// Removes the assignment of a deleted device group. Call in the transaction that deletes it.
pub async fn forget_device_group(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::DeviceGroup(id)).await.map(drop)
}

/// Removes the assignment of a deleted device (`devices.id`). Call in the transaction that
/// deletes it.
pub async fn forget_device(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::Device(id)).await.map(drop)
}

/// Number of rows removed.
async fn forget_target(tx: &impl ConnectionTrait, target: Target) -> Result<u64, DbErr> {
    let result = strategy_assignment::Entity::delete_many()
        .filter(target_filter(target))
        .exec(tx)
        .await?;
    Ok(result.rows_affected)
}

/// The row of `target`: its type, and its id (NULL for the global target).
fn target_filter(target: Target) -> Condition {
    let kind = Condition::all().add(strategy_assignment::Column::TargetType.eq(target.kind()));
    match target.id() {
        Some(id) => kind.add(strategy_assignment::Column::TargetId.eq(id)),
        None => kind.add(strategy_assignment::Column::TargetId.is_null()),
    }
}

/// The strategy exists, and keeps existing until the transaction ends (shared row lock on
/// PostgreSQL; SQLite's write transaction excludes other writers).
async fn ensure_strategy(tx: &impl ConnectionTrait, id: i64) -> Result<(), StrategyError> {
    let found = strategy::Entity::find_by_id(id)
        .lock_shared()
        .one(tx)
        .await?;
    found.map(drop).ok_or(StrategyError::NotFound)
}

/// The target exists (and cannot be deleted before commit). A deletion path removes its
/// assignments in its own transaction, so no assignment of a vanished target survives.
async fn ensure_target(tx: &impl ConnectionTrait, target: Target) -> Result<(), StrategyError> {
    let found = match target {
        Target::Global => return Ok(()),
        Target::User(id) => user::Entity::find_by_id(id)
            .lock_shared()
            .one(tx)
            .await?
            .is_some(),
        Target::Group(id) => group::Entity::find_by_id(id)
            .lock_shared()
            .one(tx)
            .await?
            .is_some(),
        Target::DeviceGroup(id) => {
            let found = device_group::Entity::find_by_id(id)
                .lock_shared()
                .one(tx)
                .await?;
            found.is_some()
        }
        Target::Device(id) => device::Entity::find_by_id(id)
            .lock_shared()
            .one(tx)
            .await?
            .is_some(),
    };
    match found {
        true => Ok(()),
        false => Err(StrategyError::TargetNotFound(target)),
    }
}

/// Points the target's row at `strategy_id`, inserting it if there is none. Never relies on
/// catching a unique violation: that would abort the PostgreSQL transaction. A concurrent
/// insert of the same target makes ours do nothing and the second update succeed.
async fn upsert(tx: &impl ConnectionTrait, target: Target, strategy_id: i64) -> Result<(), DbErr> {
    if repoint(tx, target, strategy_id).await? {
        return Ok(());
    }
    let inserted = insert_if_absent(target, strategy_id)
        .exec_without_returning(tx)
        .await?;
    if matches!(inserted, TryInsertResult::Inserted(n) if n > 0)
        || repoint(tx, target, strategy_id).await?
    {
        return Ok(());
    }
    Err(DbErr::Custom(format!(
        "assignment of {target:?} vanished during upsert"
    )))
}

/// `INSERT ... ON CONFLICT DO NOTHING` **without a conflict target**, so any unique index
/// absorbs the conflict: `uq_strategy_assignments_target` for the four id-carrying targets
/// and the partial `uq_strategy_assignments_global` for the global one. sea-orm's own
/// `on_conflict_do_nothing()` targets the primary key (`("id")`), which neither index is,
/// so a concurrent first insert of the same target would raise 23505 and abort the
/// PostgreSQL transaction.
fn insert_if_absent(
    target: Target,
    strategy_id: i64,
) -> TryInsert<strategy_assignment::ActiveModel> {
    let row = strategy_assignment::ActiveModel {
        id: NotSet,
        strategy_id: Set(strategy_id),
        target_type: Set(target.kind()),
        target_id: Set(target.id()),
    };
    strategy_assignment::Entity::insert(row)
        .on_conflict(OnConflict::new().do_nothing().to_owned())
        .try_insert()
}

/// `true` if the target had a row (now pointing at `strategy_id`).
async fn repoint(
    tx: &impl ConnectionTrait,
    target: Target,
    strategy_id: i64,
) -> Result<bool, DbErr> {
    let updated = strategy_assignment::Entity::update_many()
        .col_expr(
            strategy_assignment::Column::StrategyId,
            Expr::value(strategy_id),
        )
        .filter(target_filter(target))
        .exec(tx)
        .await?;
    Ok(updated.rows_affected > 0)
}

#[cfg(test)]
mod tests {
    use sea_orm::{DbBackend, QueryTrait};

    use super::*;

    #[test]
    fn the_insert_has_no_conflict_target_on_either_backend() {
        for target in [Target::Global, Target::User(7)] {
            for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
                let built = insert_if_absent(target, 1).build(backend).to_string();
                // sea-query leaves two spaces where the conflict target would be.
                let sql = built.split_whitespace().collect::<Vec<_>>().join(" ");

                assert!(
                    sql.ends_with(" ON CONFLICT DO NOTHING"),
                    "{backend:?}: {sql}"
                );
                assert!(!sql.contains("ON CONFLICT (\"id\")"), "{backend:?}: {sql}");
            }
        }
    }
}
