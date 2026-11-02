#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_core::strategies::{Assignment, StrategyError, Target};
use deskhaven_db::entity::strategy_assignment;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};
use tokio::sync::Barrier;

use super::support::ctx;

async fn rows(db: &TestDb) -> u64 {
    strategy_assignment::Entity::find()
        .count(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn assigning_replaces_the_strategy_of_a_target() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (a, b) = (c.strategy("a", &[]).await, c.strategy("b", &[]).await);
        let user = c.user("alice").await;
        let group = c.group("ops").await;
        let device_group = c.device_group("lab").await;
        let device = c.device("100").await;
        let targets = [
            Target::Global,
            Target::Group(group.id),
            Target::User(user.id),
            Target::DeviceGroup(device_group),
            Target::Device(device),
        ];

        for target in targets {
            assert_eq!(c.svc.assignment(target).await.unwrap(), None);
            c.assign(target, &a).await;
            assert_eq!(
                c.svc.assignment(target).await.unwrap(),
                Some(a.id),
                "{target:?}"
            );
            c.assign(target, &b).await;
            assert_eq!(
                c.svc.assignment(target).await.unwrap(),
                Some(b.id),
                "{target:?}"
            );
            // Assigning what is already assigned is fine.
            c.assign(target, &b).await;
        }

        assert_eq!(rows(db).await, 5, "one row per target");
        let all = c.svc.assignments().await.unwrap();
        assert!(all.iter().all(|x| x.strategy_id == b.id));
        assert_eq!(all.len(), 5);
    })
    .await;
}

#[tokio::test]
async fn the_same_strategy_may_serve_many_targets() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let a = c.strategy("a", &[("k", "v")]).await;
        let (d1, d2) = (c.device("1").await, c.device("2").await);

        c.assign(Target::Device(d1), &a).await;
        c.assign(Target::Device(d2), &a).await;
        c.assign(Target::Global, &a).await;

        assert_eq!(rows(db).await, 3);
        let expected = vec![
            Assignment {
                target: Target::Global,
                strategy_id: a.id,
            },
            Assignment {
                target: Target::Device(d1),
                strategy_id: a.id,
            },
            Assignment {
                target: Target::Device(d2),
                strategy_id: a.id,
            },
        ];
        assert_eq!(c.svc.assignments().await.unwrap(), expected);
    })
    .await;
}

#[tokio::test]
async fn there_is_one_global_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (a, b) = (
            c.strategy("a", &[("k", "a")]).await,
            c.strategy("b", &[("k", "b")]).await,
        );

        c.assign(Target::Global, &a).await;
        c.assign(Target::Global, &b).await;

        assert_eq!(rows(db).await, 1);
        assert_eq!(c.svc.assignment(Target::Global).await.unwrap(), Some(b.id));
        assert!(c.svc.unassign(Target::Global).await.unwrap());
        assert_eq!(rows(db).await, 0);
    })
    .await;
}

/// Rounds of the concurrency test. With the primary-key conflict target the race failed on
/// PostgreSQL in roughly one run in five, so 50 rounds make a regression near-certain.
const RACE_ROUNDS: usize = 50;
const RACERS: usize = 6;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_assigns_to_one_target_never_fail_and_leave_one_row() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let mut strategies = Vec::new();
        for i in 0..RACERS {
            strategies.push(c.strategy(&format!("s{i}"), &[]).await);
        }
        let device = c.device("1").await;
        let ids: Vec<_> = strategies.iter().map(|s| Some(s.id)).collect();

        for round in 0..RACE_ROUNDS {
            // Fresh targets each round: the race is between *first* assigns.
            c.svc.unassign(Target::Global).await.unwrap();
            c.svc.unassign(Target::Device(device)).await.unwrap();
            let start = Arc::new(Barrier::new(RACERS));
            let mut tasks = Vec::new();
            for strategy in &strategies {
                let (svc, id, start) = (c.svc.clone(), strategy.id, Arc::clone(&start));
                tasks.push(tokio::spawn(async move {
                    start.wait().await;
                    let global = svc.assign(Target::Global, id).await;
                    global.and(svc.assign(Target::Device(device), id).await)
                }));
            }
            for task in tasks {
                let result = task.await.unwrap();
                assert!(result.is_ok(), "round {round}: {result:?}");
            }

            assert_eq!(rows(db).await, 2, "round {round}");
            assert!(ids.contains(&c.svc.assignment(Target::Global).await.unwrap()));
            assert!(ids.contains(&c.svc.assignment(Target::Device(device)).await.unwrap()));
        }
    })
    .await;
}

#[tokio::test]
async fn unassign_reports_whether_anything_was_removed() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let a = c.strategy("a", &[]).await;
        let user = c.user("alice").await;
        c.assign(Target::User(user.id), &a).await;

        assert!(c.svc.unassign(Target::User(user.id)).await.unwrap());
        assert!(!c.svc.unassign(Target::User(user.id)).await.unwrap());
        assert!(!c.svc.unassign(Target::Global).await.unwrap());
        assert_eq!(rows(db).await, 0);
        // The strategy itself stays.
        assert!(c.svc.get(a.id).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn assigning_requires_an_existing_strategy_and_target() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let a = c.strategy("a", &[]).await;

        let missing = c.svc.assign(Target::Global, 9999).await;
        assert!(
            matches!(missing, Err(StrategyError::NotFound)),
            "{missing:?}"
        );
        for target in [
            Target::User(9999),
            Target::Group(9999),
            Target::DeviceGroup(9999),
            Target::Device(9999),
        ] {
            let result = c.svc.assign(target, a.id).await;
            assert!(
                matches!(result, Err(StrategyError::TargetNotFound(t)) if t == target),
                "{result:?}"
            );
        }
        assert_eq!(rows(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_strategy_removes_its_assignments() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (a, b) = (
            c.strategy("a", &[("k", "a")]).await,
            c.strategy("b", &[]).await,
        );
        let device = c.device("1").await;
        c.assign(Target::Global, &a).await;
        c.assign(Target::Device(device), &a).await;
        c.assign(Target::User(c.user("alice").await.id), &b).await;

        c.svc.delete(a.id).await.unwrap();

        let left = c.svc.assignments().await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].strategy_id, b.id);
        let inputs = super::support::device_inputs(device, None, None);
        assert!(c.options(&inputs).await.is_empty());
    })
    .await;
}

/// `assign_if_unassigned` (device presets, M2-12) fills an empty target only, and only then
/// invalidates the cache.
#[tokio::test]
async fn assign_if_unassigned_never_replaces_an_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let (a, b) = (c.strategy("a", &[]).await, c.strategy("b", &[]).await);
        let device = c.device("123456789").await;
        let target = Target::Device(device);

        // Act
        let first = c.svc.assign_if_unassigned(target, a.id).await.unwrap();
        c.svc.effective(&Default::default()).await.unwrap();
        let second = c.svc.assign_if_unassigned(target, b.id).await.unwrap();
        let kept_cache = c.svc.cache().is_loaded();
        let missing = c
            .svc
            .assign_if_unassigned(Target::Device(device + 100), a.id)
            .await;

        // Assert
        let be = db.backend;
        assert!(first && !second, "{be}");
        assert_eq!(c.svc.assignment(target).await.unwrap(), Some(a.id), "{be}");
        assert!(kept_cache, "{be}: nothing changed, nothing invalidated");
        assert!(
            matches!(missing, Err(StrategyError::TargetNotFound(_))),
            "{be}: {missing:?}"
        );
        assert_eq!(rows(db).await, 1, "{be}");
    })
    .await;
}
