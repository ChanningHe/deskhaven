//! Strategies and their assignments (04-data-model §6).
//!
//! Each target has at most one strategy: `UNIQUE(target_type, target_id)`. The `global`
//! target has no id, and NULLs never conflict in a unique index, so a second partial unique
//! index over `target_type` for the global row keeps it single; a CHECK ties the NULL id to
//! the `global` type. `target_id` points at a device, device group, user or group depending
//! on `target_type`, so it has no foreign key; the strategy service removes assignments of
//! deleted targets. The table gets a surrogate `id` (the logical model has no key because
//! `target_id` is nullable).

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, big_pk_auto, text};

use super::{enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Strategies {
    Table,
    Id,
    Name,
    Options,
    Note,
    ManagedBy,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum StrategyAssignments {
    Table,
    Id,
    StrategyId,
    TargetType,
    TargetId,
}

const MANAGED_BY: &[&str] = &["ui", "config"];
const TARGET_TYPES: &[&str] = &["device", "device_group", "user", "group", "global"];
const GLOBAL: &str = "global";
const EMPTY_JSON_OBJECT: &str = "{}";

fn strategies_table() -> TableCreateStatement {
    Table::create()
        .table(Strategies::Table)
        .col(big_pk_auto(Strategies::Id))
        .col(text(Strategies::Name))
        .col(text(Strategies::Options).default(EMPTY_JSON_OBJECT))
        .col(text(Strategies::Note).default(""))
        .col(text(Strategies::ManagedBy).default("ui"))
        .col(big_integer(Strategies::CreatedAt))
        .col(big_integer(Strategies::UpdatedAt))
        .check(enum_check(
            "ck_strategies_managed_by",
            Strategies::ManagedBy,
            MANAGED_BY,
        ))
        .to_owned()
}

/// `target_id` is NULL exactly for the global target.
fn global_has_no_id() -> Expr {
    let is_global = Expr::col(StrategyAssignments::TargetType).eq(GLOBAL);
    is_global
        .clone()
        .and(Expr::col(StrategyAssignments::TargetId).is_null())
        .or(Expr::col(StrategyAssignments::TargetType)
            .ne(GLOBAL)
            .and(Expr::col(StrategyAssignments::TargetId).is_not_null()))
}

fn assignments_table() -> TableCreateStatement {
    Table::create()
        .table(StrategyAssignments::Table)
        .col(big_pk_auto(StrategyAssignments::Id))
        .col(big_integer(StrategyAssignments::StrategyId))
        .col(text(StrategyAssignments::TargetType))
        .col(big_integer_null(StrategyAssignments::TargetId))
        .check(enum_check(
            "ck_strategy_assignments_target_type",
            StrategyAssignments::TargetType,
            TARGET_TYPES,
        ))
        .check(Check::named(
            "ck_strategy_assignments_target_id",
            global_has_no_id(),
        ))
        .foreign_key(&mut foreign_key(
            "fk_strategy_assignments_strategy",
            (StrategyAssignments::Table, StrategyAssignments::StrategyId),
            (Strategies::Table, Strategies::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn indexes() -> Vec<IndexCreateStatement> {
    vec![
        unique_index("uq_strategies_name", Strategies::Table, Strategies::Name),
        Index::create()
            .name("uq_strategy_assignments_target")
            .table(StrategyAssignments::Table)
            .col(StrategyAssignments::TargetType)
            .col(StrategyAssignments::TargetId)
            .unique()
            .to_owned(),
        // NULL target ids never conflict, so the single global row needs its own index.
        Index::create()
            .name("uq_strategy_assignments_global")
            .table(StrategyAssignments::Table)
            .col(StrategyAssignments::TargetType)
            .unique()
            .and_where(Expr::col(StrategyAssignments::TargetType).eq(GLOBAL))
            .to_owned(),
        index(
            "idx_strategy_assignments_strategy_id",
            StrategyAssignments::Table,
            StrategyAssignments::StrategyId,
        ),
    ]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(strategies_table()).await?;
        manager.create_table(assignments_table()).await?;
        for statement in indexes() {
            manager.create_index(statement).await?;
        }
        Ok(())
    }
}
