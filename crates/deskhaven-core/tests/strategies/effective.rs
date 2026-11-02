//! Effective strategy through the service: real assignments, memberships and ownership.
//! The merge rules themselves are unit-tested next to the code.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::{Assignment, Target};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

use super::support::{ctx, device_inputs, opts};

#[tokio::test]
async fn all_five_layers_merge_in_priority_order() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let ops = c.group("ops").await;
        c.groups.add_member(ops.id, alice.id).await.unwrap();
        let lab = c.device_group("lab").await;
        let device = c.device("100").await;
        let global = c
            .strategy(
                "global",
                &[("a", "global"), ("b", "global"), ("c", "global")],
            )
            .await;
        let group = c
            .strategy("group", &[("b", "group"), ("c", "group"), ("d", "group")])
            .await;
        let owner = c
            .strategy("owner", &[("c", "owner"), ("d", "owner"), ("e", "owner")])
            .await;
        let dgroup = c
            .strategy(
                "dgroup",
                &[("d", "dgroup"), ("e", "dgroup"), ("f", "dgroup")],
            )
            .await;
        let direct = c
            .strategy("direct", &[("e", "direct"), ("f", "direct")])
            .await;
        c.assign(Target::Global, &global).await;
        c.assign(Target::Group(ops.id), &group).await;
        c.assign(Target::User(alice.id), &owner).await;
        c.assign(Target::DeviceGroup(lab), &dgroup).await;
        c.assign(Target::Device(device), &direct).await;

        let effective = c
            .svc
            .effective(&device_inputs(device, Some(&alice), Some(lab)))
            .await
            .unwrap();

        let expected = opts(&[
            ("a", "global"),
            ("b", "group"),
            ("c", "owner"),
            ("d", "dgroup"),
            ("e", "direct"),
            ("f", "direct"),
        ]);
        assert_eq!(effective.options, expected);
        let applied: Vec<_> = effective.applied.iter().map(|a| a.target).collect();
        let order = [
            Target::Global,
            Target::Group(ops.id),
            Target::User(alice.id),
            Target::DeviceGroup(lab),
            Target::Device(device),
        ];
        assert_eq!(applied, order);

        // Without an owner or a device group only the global and direct layers remain.
        let bare = c
            .svc
            .effective(&device_inputs(device, None, None))
            .await
            .unwrap();
        let bare_targets: Vec<_> = bare.applied.iter().map(|a| a.target).collect();
        assert_eq!(bare_targets, [Target::Global, Target::Device(device)]);
    })
    .await;
}

#[tokio::test]
async fn empty_values_delete_on_the_client_and_can_be_overridden_back() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let device = c.device("1").await;
        let global = c
            .strategy("global", &[("keep", "v"), ("drop", "v"), ("flip", "")])
            .await;
        let direct = c.strategy("direct", &[("drop", ""), ("flip", "on")]).await;
        c.assign(Target::Global, &global).await;
        c.assign(Target::Device(device), &direct).await;

        let effective = c
            .svc
            .effective(&device_inputs(device, Some(&alice), None))
            .await
            .unwrap();

        // "drop" stays in the wire map as "": the client deletes it.
        assert_eq!(
            effective.options,
            opts(&[("keep", "v"), ("drop", ""), ("flip", "on")])
        );
        let live: Vec<_> = effective.live_options().collect();
        assert_eq!(live, [("flip", "on"), ("keep", "v")]);
    })
    .await;
}

#[tokio::test]
async fn an_owner_in_several_assigned_groups_gets_the_higher_group_id_on_ties() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let bob = c.user("bob").await;
        let (first, second) = (c.group("first").await, c.group("second").await);
        let a = c
            .strategy("a", &[("k", "first"), ("only-first", "1")])
            .await;
        let b = c.strategy("b", &[("k", "second")]).await;
        // Assignment order must not matter.
        c.assign(Target::Group(second.id), &b).await;
        c.assign(Target::Group(first.id), &a).await;
        c.groups.add_member(first.id, alice.id).await.unwrap();
        c.groups.add_member(second.id, alice.id).await.unwrap();
        c.groups.add_member(first.id, bob.id).await.unwrap();

        let both = c.options(&device_inputs(1, Some(&alice), None)).await;
        let only_first = c.options(&device_inputs(1, Some(&bob), None)).await;

        assert_eq!(both, opts(&[("k", "second"), ("only-first", "1")]));
        assert_eq!(only_first, opts(&[("k", "first"), ("only-first", "1")]));
    })
    .await;
}

#[tokio::test]
async fn devices_without_applicable_assignments_get_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let bob = c.user("bob").await;
        let s = c.strategy("s", &[("k", "v")]).await;
        c.assign(Target::User(alice.id), &s).await;

        let effective = c
            .svc
            .effective(&device_inputs(5, Some(&bob), Some(77)))
            .await
            .unwrap();

        assert!(effective.options.is_empty());
        assert!(effective.applied.is_empty());
        let mine = c
            .svc
            .effective(&device_inputs(5, Some(&alice), None))
            .await
            .unwrap();
        assert_eq!(
            mine.applied,
            [Assignment {
                target: Target::User(alice.id),
                strategy_id: s.id
            }]
        );
    })
    .await;
}
