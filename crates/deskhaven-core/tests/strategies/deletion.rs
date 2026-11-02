//! Deleting a user or user group removes its strategy assignment in the same transaction.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::strategies::Target;
use deskhaven_db::entity::strategy_assignment;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

use super::support::ctx;

async fn rows(db: &TestDb) -> u64 {
    strategy_assignment::Entity::find()
        .count(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_a_user_removes_only_its_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (alice, bob) = (c.user("alice").await, c.user("bob").await);
        let s = c.strategy("s", &[]).await;
        c.assign(Target::User(alice.id), &s).await;
        c.assign(Target::User(bob.id), &s).await;
        c.assign(Target::Global, &s).await;

        c.users.delete(alice.id).await.unwrap();

        assert_eq!(
            c.svc.assignment(Target::User(alice.id)).await.unwrap(),
            None
        );
        assert_eq!(
            c.svc.assignment(Target::User(bob.id)).await.unwrap(),
            Some(s.id)
        );
        assert_eq!(rows(db).await, 2);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_removes_only_its_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (ops, dev) = (c.group("ops").await, c.group("dev").await);
        let s = c.strategy("s", &[]).await;
        c.assign(Target::Group(ops.id), &s).await;
        c.assign(Target::Group(dev.id), &s).await;

        c.groups.delete(ops.id).await.unwrap();

        assert_eq!(c.svc.assignment(Target::Group(ops.id)).await.unwrap(), None);
        assert_eq!(
            c.svc.assignment(Target::Group(dev.id)).await.unwrap(),
            Some(s.id)
        );
    })
    .await;
}

#[tokio::test]
async fn an_id_is_never_confused_across_target_types() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let alice = c.user("alice").await;
        let s = c.strategy("s", &[]).await;
        // Same numeric id as the user, different target type.
        let group = c.group("ops").await;
        let device_group = c.device_group("lab").await;
        c.assign(Target::Group(group.id), &s).await;
        c.assign(Target::DeviceGroup(device_group), &s).await;

        c.users.delete(alice.id).await.unwrap();

        assert_eq!(rows(db).await, 2);
    })
    .await;
}
