//! Strategy schema (M2-02): one strategy per target, including the single global assignment
//! (a partial unique index, because NULL target ids never conflict), on every available
//! backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::count;
use common::ops::{
    assert_checks, assert_indexes, assignment_row, is_fk_violation, is_unique_violation,
    strategy_row,
};
use deskhaven_db::entity::strategy_assignment::TargetType;
use deskhaven_db::entity::{strategy, strategy_assignment};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

#[tokio::test]
async fn tables_and_indexes_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let manager = SchemaManager::new(&db.conn);
        for table in ["strategies", "strategy_assignments"] {
            assert!(
                manager.has_table(table).await.unwrap(),
                "{}: missing {table}",
                db.backend
            );
        }
        assert_indexes(
            db,
            &[
                ("strategies", "uq_strategies_name"),
                ("strategy_assignments", "uq_strategy_assignments_target"),
                ("strategy_assignments", "uq_strategy_assignments_global"),
                (
                    "strategy_assignments",
                    "idx_strategy_assignments_strategy_id",
                ),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn strategy_name_is_unique_and_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = strategy_row("kiosk").insert(&db.conn).await.unwrap();

        let dup = strategy_row("kiosk").insert(&db.conn).await;
        let found = strategy::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();

        assert!(is_unique_violation(dup), "{}", db.backend);
        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn managed_by_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        strategy_row("kiosk").insert(&db.conn).await.unwrap();

        assert_checks(
            db,
            &[("strategies", "managed_by = 'both'", "managed_by = 'config'")],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn each_target_has_at_most_one_strategy() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let a = strategy_row("a").insert(conn).await.unwrap();
        let b = strategy_row("b").insert(conn).await.unwrap();
        assignment_row(a.id, TargetType::Device, Some(5))
            .insert(conn)
            .await
            .unwrap();

        let same_strategy = assignment_row(a.id, TargetType::Device, Some(5))
            .insert(conn)
            .await;
        let other_strategy = assignment_row(b.id, TargetType::Device, Some(5))
            .insert(conn)
            .await;
        // Same numeric id, other target type: a different target.
        let other_type = assignment_row(b.id, TargetType::User, Some(5))
            .insert(conn)
            .await;

        let w = db.backend;
        assert!(
            is_unique_violation(same_strategy),
            "{w}: same strategy twice"
        );
        assert!(is_unique_violation(other_strategy), "{w}: second strategy");
        assert!(other_type.is_ok(), "{w}: user 5 is not device 5");
    })
    .await;
}

#[tokio::test]
async fn a_strategy_may_have_many_targets() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let a = strategy_row("a").insert(conn).await.unwrap();

        for (kind, id) in [
            (TargetType::Device, 1),
            (TargetType::DeviceGroup, 1),
            (TargetType::User, 1),
            (TargetType::Group, 1),
        ] {
            let res = assignment_row(a.id, kind, Some(id)).insert(conn).await;
            assert!(res.is_ok(), "{}: {kind:?}", db.backend);
        }
        let global = assignment_row(a.id, TargetType::Global, None)
            .insert(conn)
            .await;

        assert!(global.is_ok(), "{}: global", db.backend);
        assert_eq!(count::<strategy_assignment::Entity>(conn).await, 5);
    })
    .await;
}

#[tokio::test]
async fn at_most_one_global_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let a = strategy_row("a").insert(conn).await.unwrap();
        let b = strategy_row("b").insert(conn).await.unwrap();
        assignment_row(a.id, TargetType::Global, None)
            .insert(conn)
            .await
            .unwrap();

        let other_strategy = assignment_row(b.id, TargetType::Global, None)
            .insert(conn)
            .await;
        let same_strategy = assignment_row(a.id, TargetType::Global, None)
            .insert(conn)
            .await;

        let w = db.backend;
        assert!(is_unique_violation(other_strategy), "{w}: second global");
        assert!(is_unique_violation(same_strategy), "{w}: duplicate global");
    })
    .await;
}

#[tokio::test]
async fn target_type_and_target_id_are_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let a = strategy_row("a").insert(&db.conn).await.unwrap();
        assignment_row(a.id, TargetType::Device, Some(5))
            .insert(&db.conn)
            .await
            .unwrap();

        assert_checks(
            db,
            &[
                (
                    "strategy_assignments",
                    "target_type = 'everyone'",
                    "target_type = 'user'",
                ),
                // Only the global target has no id; every other target needs one.
                ("strategy_assignments", "target_id = NULL", "target_id = 6"),
                (
                    "strategy_assignments",
                    "target_type = 'global'",
                    "target_type = 'global', target_id = NULL",
                ),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn assignments_reject_an_unknown_strategy() {
    each_migrated_backend(async |db: &TestDb| {
        let res = assignment_row(404, TargetType::Global, None)
            .insert(&db.conn)
            .await;

        assert!(is_fk_violation(res), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_strategy_removes_only_its_assignments() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let a = strategy_row("a").insert(conn).await.unwrap();
        let b = strategy_row("b").insert(conn).await.unwrap();
        assignment_row(a.id, TargetType::Device, Some(1))
            .insert(conn)
            .await
            .unwrap();
        assignment_row(a.id, TargetType::Global, None)
            .insert(conn)
            .await
            .unwrap();
        assignment_row(b.id, TargetType::Device, Some(2))
            .insert(conn)
            .await
            .unwrap();

        strategy::Entity::delete_by_id(a.id)
            .exec(conn)
            .await
            .unwrap();

        let left = strategy_assignment::Entity::find().all(conn).await.unwrap();
        assert_eq!(left.len(), 1, "{}", db.backend);
        assert_eq!(left[0].strategy_id, b.id, "{}", db.backend);
    })
    .await;
}
