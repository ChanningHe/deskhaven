//! `target_id` has no foreign key (it points at four tables), so the deletion paths call the
//! `forget_*` functions in their own transaction and invalidate the cache after commit.

use deskhaven_db::begin_write;
use deskhaven_db::entity::managed_by::ManagedBy;
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
use crate::console_audit::TxGuard;

impl StrategyService {
    /// Assigning the strategy it already has still advances the version.
    pub async fn assign(&self, target: Target, strategy_id: i64) -> Result<(), StrategyError> {
        self.assign_with(target, strategy_id, TxGuard::none(), false)
            .await
    }

    pub async fn assign_guarded(
        &self,
        target: Target,
        strategy_id: i64,
        guard: TxGuard<Assignment>,
    ) -> Result<(), StrategyError> {
        self.assign_with(target, strategy_id, guard, true).await
    }

    async fn assign_with(
        &self,
        target: Target,
        strategy_id: i64,
        guard: TxGuard<Assignment>,
        console: bool,
    ) -> Result<(), StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let managed = ensure_strategy(&tx, strategy_id).await?;
            ensure_target(&tx, target).await?;
            let replaced = assignment_in(&tx, target).await?;
            if console {
                refuse_config(managed)?;
                if let Some(old) = replaced {
                    refuse_config(ensure_strategy(&tx, old).await?)?;
                }
            }
            let before = replaced.map(|strategy_id| Assignment {
                target,
                strategy_id,
            });
            upsert(&tx, target, strategy_id).await?;
            let after = Assignment {
                target,
                strategy_id,
            };
            guard.check(&tx, before.as_ref(), Some(&after)).await?;
            tx.commit().await?;
            inner.cache.invalidate();
            Ok(())
        })
        .await
    }

    pub async fn unassign_guarded(
        &self,
        target: Target,
        strategy_id: i64,
        guard: TxGuard<Assignment>,
    ) -> Result<bool, StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            refuse_config(ensure_strategy(&tx, strategy_id).await?)?;
            let removed = strategy_assignment::Entity::delete_many()
                .filter(target_filter(target))
                .filter(strategy_assignment::Column::StrategyId.eq(strategy_id))
                .exec(&tx)
                .await?;
            if removed.rows_affected == 0 {
                return Ok(false);
            }
            let before = Assignment {
                target,
                strategy_id,
            };
            guard.check(&tx, Some(&before), None).await?;
            tx.commit().await?;
            inner.cache.invalidate();
            Ok(true)
        })
        .await
    }

    /// Never replaces an administrator's choice. Atomic: a concurrent [`assign`](Self::assign) either
    /// comes first and wins, or replaces this one.
    pub async fn assign_if_unassigned(
        &self,
        target: Target,
        strategy_id: i64,
    ) -> Result<bool, StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            ensure_strategy(&tx, strategy_id).await?;
            ensure_target(&tx, target).await?;
            let inserted = insert_if_absent(target, strategy_id)
                .exec_without_returning(&tx)
                .await?;
            let assigned = matches!(inserted, TryInsertResult::Inserted(n) if n > 0);
            tx.commit().await?;
            if assigned {
                inner.cache.invalidate();
            }
            Ok(assigned)
        })
        .await
    }

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

    pub async fn assignment(&self, target: Target) -> Result<Option<i64>, StrategyError> {
        let row = strategy_assignment::Entity::find()
            .filter(target_filter(target))
            .one(&self.inner.db)
            .await?;
        Ok(row.map(|r| r.strategy_id))
    }

    pub async fn assignments(&self) -> Result<Vec<Assignment>, StrategyError> {
        let mut all = load::assignments(&self.inner.db).await?;
        all.sort_by_key(|a| a.target);
        Ok(all)
    }
}

/// The caller invalidates the cache after commit.
pub(crate) async fn assign_in(
    tx: &impl ConnectionTrait,
    target: Target,
    strategy_id: i64,
) -> Result<(), StrategyError> {
    ensure_strategy(tx, strategy_id).await?;
    ensure_target(tx, target).await?;
    upsert(tx, target, strategy_id).await?;
    Ok(())
}

pub(crate) async fn assignment_in(
    conn: &impl ConnectionTrait,
    target: Target,
) -> Result<Option<i64>, DbErr> {
    let row = strategy_assignment::Entity::find()
        .filter(target_filter(target))
        .one(conn)
        .await?;
    Ok(row.map(|r| r.strategy_id))
}

pub(crate) async fn id_by_name_in(
    tx: &impl ConnectionTrait,
    name: &str,
) -> Result<Option<i64>, DbErr> {
    let found = strategy::Entity::find()
        .filter(strategy::Column::Name.eq(name.trim()))
        .lock_shared()
        .one(tx)
        .await?;
    Ok(found.map(|s| s.id))
}

pub async fn forget_user(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::User(id)).await.map(drop)
}

pub async fn forget_group(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::Group(id)).await.map(drop)
}

pub async fn forget_device_group(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::DeviceGroup(id)).await.map(drop)
}

pub async fn forget_device(tx: &impl ConnectionTrait, id: i64) -> Result<(), DbErr> {
    forget_target(tx, Target::Device(id)).await.map(drop)
}

async fn forget_target(tx: &impl ConnectionTrait, target: Target) -> Result<u64, DbErr> {
    let result = strategy_assignment::Entity::delete_many()
        .filter(target_filter(target))
        .exec(tx)
        .await?;
    Ok(result.rows_affected)
}

fn target_filter(target: Target) -> Condition {
    let kind = Condition::all().add(strategy_assignment::Column::TargetType.eq(target.kind()));
    match target.id() {
        Some(id) => kind.add(strategy_assignment::Column::TargetId.eq(id)),
        None => kind.add(strategy_assignment::Column::TargetId.is_null()),
    }
}

/// Shared row lock on PostgreSQL; SQLite's write transaction excludes other writers.
async fn ensure_strategy(tx: &impl ConnectionTrait, id: i64) -> Result<ManagedBy, StrategyError> {
    let found = strategy::Entity::find_by_id(id)
        .lock_shared()
        .one(tx)
        .await?;
    found.map(|s| s.managed_by).ok_or(StrategyError::NotFound)
}

fn refuse_config(managed: ManagedBy) -> Result<(), StrategyError> {
    match managed {
        ManagedBy::Config => Err(StrategyError::ManagedByConfig),
        ManagedBy::Ui => Ok(()),
    }
}

/// A deletion path removes its assignments in its own transaction, so none of a vanished target
/// survives.
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

/// Never relies on catching a unique violation: that would abort the PostgreSQL transaction.
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

/// No conflict target, so any unique index absorbs the conflict; sea-orm's
/// `on_conflict_do_nothing()` targets the primary key, so a concurrent first insert would raise
/// 23505 and abort the transaction.
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
