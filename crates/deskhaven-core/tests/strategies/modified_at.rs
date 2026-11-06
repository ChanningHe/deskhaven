//! `modified_at`: the version clients compare. Strictly increasing on every relevant change,
//! stable otherwise, never taken from the database's idea of "newest input".

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::strategies::{DeviceInputs, StrategyService, StrategyUpdate, Target};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;

use super::support::{T0_MS, ctx, device_inputs, opts};

async fn version(svc: &StrategyService, inputs: &DeviceInputs) -> i64 {
    svc.effective(inputs).await.unwrap().modified_at
}

#[tokio::test]
async fn a_new_service_starts_at_the_clock_and_reads_leave_it_alone() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let inputs = device_inputs(1, None, None);
        let s = c.strategy("s", &[]).await;

        assert_eq!(version(&c.svc, &inputs).await, T0_MS);
        assert_eq!(c.svc.modified_at(&inputs), T0_MS);
        c.clock.advance(Duration::from_secs(60));
        c.svc.list().await.unwrap();
        c.svc.get(s.id).await.unwrap();
        c.svc.assignments().await.unwrap();
        // Creating a strategy, or a failed write, is not a change to any device.
        c.strategy("other", &[]).await;
        assert!(c.svc.assign(Target::Global, 9999).await.is_err());
        assert_eq!(version(&c.svc, &inputs).await, T0_MS);
    })
    .await;
}

#[tokio::test]
async fn every_change_advances_the_version_even_when_the_clock_stands_still() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let inputs = device_inputs(device, None, None);
        let s = c.strategy("s", &[("k", "1")]).await;
        let mut seen = vec![version(&c.svc, &inputs).await];

        c.assign(Target::Device(device), &s).await;
        seen.push(version(&c.svc, &inputs).await);
        let update = StrategyUpdate {
            options: Some(opts(&[("k", "2")])),
            ..StrategyUpdate::default()
        };
        c.svc.update(s.id, update).await.unwrap();
        seen.push(version(&c.svc, &inputs).await);
        c.svc.unassign(Target::Device(device)).await.unwrap();
        seen.push(version(&c.svc, &inputs).await);
        c.assign(Target::Global, &s).await;
        seen.push(version(&c.svc, &inputs).await);
        c.svc.delete(s.id).await.unwrap();
        seen.push(version(&c.svc, &inputs).await);

        assert!(seen.windows(2).all(|w| w[0] < w[1]), "{seen:?}");
        assert_eq!(seen[0], T0_MS);
    })
    .await;
}

#[tokio::test]
async fn the_version_follows_the_clock_when_it_moves_forward_and_never_goes_back() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let inputs = device_inputs(1, None, None);
        let s = c.strategy("s", &[]).await;

        c.clock.advance(Duration::from_secs(10));
        c.assign(Target::Global, &s).await;
        let after_assign = version(&c.svc, &inputs).await;
        assert_eq!(after_assign, T0_MS + 10_000);

        // A wall clock stepped back must not lower what clients already hold.
        c.clock.set(T0_MS - 5_000);
        c.svc.unassign(Target::Global).await.unwrap();
        assert!(version(&c.svc, &inputs).await > after_assign);
    })
    .await;
}

#[tokio::test]
async fn removing_the_newest_input_still_raises_the_version() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let inputs = device_inputs(device, None, None);
        let old = c.strategy("old", &[("k", "old")]).await;
        c.clock.advance(Duration::from_secs(100));
        let newest = c.strategy("newest", &[("k", "newest")]).await;
        c.assign(Target::Global, &old).await;
        c.assign(Target::Device(device), &newest).await;
        let before = version(&c.svc, &inputs).await;

        c.clock.advance(Duration::from_secs(1));
        c.svc.unassign(Target::Device(device)).await.unwrap();

        // "Newest updated_at among the inputs" would drop back to `old`'s older time.
        assert!(version(&c.svc, &inputs).await > before);
    })
    .await;
}

#[tokio::test]
async fn a_restart_counts_as_a_change() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let inputs = device_inputs(1, None, None);
        let s = c.strategy("s", &[]).await;
        c.assign(Target::Global, &s).await;
        let before = version(&c.svc, &inputs).await;

        c.clock.advance(Duration::from_secs(30));
        let restarted = StrategyService::new(db.conn.clone(), c.crypto.clone(), c.clock.clone());

        let after = version(&restarted, &inputs).await;
        assert!(after > before);
        assert_eq!(after, T0_MS + 30_000);
    })
    .await;
}

#[tokio::test]
async fn a_device_change_stamp_raises_the_version_only_when_it_is_later() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let at = |ms: i64| DeviceInputs {
            changed_at: time::from_millis(ms),
            ..device_inputs(1, None, None)
        };

        let later = c.svc.effective(&at(T0_MS + 500)).await.unwrap().modified_at;
        let earlier = c.svc.effective(&at(T0_MS - 500)).await.unwrap().modified_at;

        assert_eq!(later, T0_MS + 500);
        assert_eq!(c.svc.modified_at(&at(T0_MS + 500)), T0_MS + 500);
        assert_eq!(earlier, T0_MS);
        assert_eq!(c.svc.modified_at(&at(T0_MS - 500)), T0_MS);
    })
    .await;
}

#[tokio::test]
async fn effective_and_the_cheap_modified_at_agree() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let inputs = device_inputs(1, None, None);
        let s = c.strategy("s", &[]).await;
        for _ in 0..3 {
            c.clock.advance(Duration::from_millis(7));
            c.assign(Target::Global, &s).await;
            assert_eq!(c.svc.modified_at(&inputs), version(&c.svc, &inputs).await);
        }
    })
    .await;
}
