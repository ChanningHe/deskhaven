//! The cache serves repeated lookups without the database and drops its snapshot after
//! every mutation that can change an effective strategy.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::{DeviceInputs, StrategyUpdate, Target};
use deskhaven_db::begin_write;
use deskhaven_db::entity::{device, device_group};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, TransactionTrait};

use super::support::{Ctx, ctx, device_inputs, opts};

/// Loads the snapshot and returns the version it carries.
async fn warm(c: &Ctx, inputs: &DeviceInputs) -> i64 {
    let version = c.svc.effective(inputs).await.unwrap().modified_at;
    assert!(c.svc.cache().is_loaded());
    version
}

/// After a mutation: the snapshot is gone and the version moved on.
fn assert_invalidated(c: &Ctx, before: i64, what: &str) {
    assert!(!c.svc.cache().is_loaded(), "{what}: snapshot kept");
    assert!(
        c.svc.cache().version() > before,
        "{what}: version did not advance"
    );
}

#[tokio::test]
async fn repeated_lookups_load_the_snapshot_once() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let s = c.strategy("s", &[("k", "v")]).await;
        c.assign(Target::Global, &s).await;
        assert_eq!(c.svc.cache().loads(), 0);

        for device in 0..5 {
            c.svc
                .effective(&device_inputs(device, None, None))
                .await
                .unwrap();
        }

        assert_eq!(c.svc.cache().loads(), 1);
        // Creating a strategy changes no effective result and keeps the snapshot.
        c.strategy("unused", &[]).await;
        assert!(c.svc.cache().is_loaded());
    })
    .await;
}

#[tokio::test]
async fn strategy_update_and_delete_invalidate_and_show_in_the_result() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let inputs = device_inputs(1, None, None);
        let s = c.strategy("s", &[("k", "old")]).await;
        c.assign(Target::Global, &s).await;
        let v0 = warm(&c, &inputs).await;

        let update = StrategyUpdate {
            options: Some(opts(&[("k", "new")])),
            ..StrategyUpdate::default()
        };
        c.svc.update(s.id, update).await.unwrap();
        assert_invalidated(&c, v0, "update");
        assert_eq!(c.options(&inputs).await, opts(&[("k", "new")]));

        let v1 = warm(&c, &inputs).await;
        let note_only = StrategyUpdate {
            note: Some("n".to_owned()),
            ..StrategyUpdate::default()
        };
        c.svc.update(s.id, note_only).await.unwrap();
        assert_invalidated(&c, v1, "note-only update");

        let v2 = warm(&c, &inputs).await;
        c.svc.delete(s.id).await.unwrap();
        assert_invalidated(&c, v2, "delete");
        assert!(c.options(&inputs).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn assignment_changes_invalidate() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let inputs = device_inputs(device, None, None);
        let (a, b) = (
            c.strategy("a", &[("k", "a")]).await,
            c.strategy("b", &[("k", "b")]).await,
        );

        let v = warm(&c, &inputs).await;
        c.assign(Target::Device(device), &a).await;
        assert_invalidated(&c, v, "assign");
        assert_eq!(c.options(&inputs).await, opts(&[("k", "a")]));

        let v = warm(&c, &inputs).await;
        c.assign(Target::Device(device), &b).await;
        assert_invalidated(&c, v, "replace");
        assert_eq!(c.options(&inputs).await, opts(&[("k", "b")]));

        let v = warm(&c, &inputs).await;
        assert!(c.svc.unassign(Target::Device(device)).await.unwrap());
        assert_invalidated(&c, v, "unassign");
        assert!(c.options(&inputs).await.is_empty());

        // Nothing to remove: nothing changed, the snapshot stays.
        let v = warm(&c, &inputs).await;
        assert!(!c.svc.unassign(Target::Device(device)).await.unwrap());
        assert!(c.svc.cache().is_loaded());
        assert_eq!(c.svc.cache().version(), v);

        // A failed assign changes nothing either.
        assert!(c.svc.assign(Target::Device(device), 9999).await.is_err());
        assert!(c.svc.cache().is_loaded());
    })
    .await;
}

#[tokio::test]
async fn membership_changes_invalidate_and_move_the_group_strategy() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let ops = c.group("ops").await;
        let s = c.strategy("s", &[("k", "ops")]).await;
        c.assign(Target::Group(ops.id), &s).await;
        let inputs = device_inputs(1, Some(&alice), None);
        assert!(c.options(&inputs).await.is_empty());

        let v = warm(&c, &inputs).await;
        c.groups.add_member(ops.id, alice.id).await.unwrap();
        assert_invalidated(&c, v, "add_member");
        assert_eq!(c.options(&inputs).await, opts(&[("k", "ops")]));

        let v = warm(&c, &inputs).await;
        c.groups.remove_member(ops.id, alice.id).await.unwrap();
        assert_invalidated(&c, v, "remove_member");
        assert!(c.options(&inputs).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_or_user_invalidates_and_drops_their_layer() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let ops = c.group("ops").await;
        c.groups.add_member(ops.id, alice.id).await.unwrap();
        let (g, u) = (
            c.strategy("g", &[("g", "1")]).await,
            c.strategy("u", &[("u", "1")]).await,
        );
        c.assign(Target::Group(ops.id), &g).await;
        c.assign(Target::User(alice.id), &u).await;
        let inputs = device_inputs(1, Some(&alice), None);
        assert_eq!(c.options(&inputs).await, opts(&[("g", "1"), ("u", "1")]));

        let v = warm(&c, &inputs).await;
        c.groups.delete(ops.id).await.unwrap();
        assert_invalidated(&c, v, "group delete");
        assert_eq!(c.options(&inputs).await, opts(&[("u", "1")]));

        // The owner is gone: the device keeps its row (owner cleared) but loses the layer.
        let v = warm(&c, &inputs).await;
        c.users.delete(alice.id).await.unwrap();
        assert_invalidated(&c, v, "user delete");
        let orphan = device_inputs(1, None, None);
        assert!(c.options(&orphan).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn device_and_device_group_deletion_paths_use_forget_and_invalidate() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let lab = c.device_group("lab").await;
        let s = c.strategy("s", &[("k", "v")]).await;
        c.assign(Target::Device(device), &s).await;
        c.assign(Target::DeviceGroup(lab), &s).await;
        let inputs = device_inputs(device, None, Some(lab));
        assert_eq!(c.options(&inputs).await, opts(&[("k", "v")]));

        // What the device registry (M2-12) does when it deletes a device.
        let tx = begin_write(&db.conn).await.unwrap();
        deskhaven_core::strategies::forget_device(&tx, device)
            .await
            .unwrap();
        device::Entity::delete_by_id(device)
            .exec(&tx)
            .await
            .unwrap();
        deskhaven_core::strategies::forget_device_group(&tx, lab)
            .await
            .unwrap();
        device_group::Entity::delete_by_id(lab)
            .exec(&tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let v = c.svc.cache().version();
        c.svc.cache().invalidate();

        assert!(!c.svc.cache().is_loaded());
        assert!(c.svc.cache().version() > v);
        assert!(c.svc.assignments().await.unwrap().is_empty());
        assert!(c.options(&inputs).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_rolled_back_forget_keeps_the_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let device = c.device("1").await;
        let s = c.strategy("s", &[]).await;
        c.assign(Target::Device(device), &s).await;

        let tx = db.conn.begin().await.unwrap();
        deskhaven_core::strategies::forget_device(&tx, device)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        assert_eq!(
            c.svc.assignment(Target::Device(device)).await.unwrap(),
            Some(s.id)
        );
    })
    .await;
}
