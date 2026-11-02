//! `device_groups` and the `devices` foreign keys (M2-02), on every available backend,
//! including the SQLite table rebuild over rows that already exist.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::ops::{
    assert_checks, assert_indexes, device_group_row, device_row, is_fk_violation,
    is_unique_violation,
};
use common::{count, user_row};
use deskhaven_db::begin_write;
use deskhaven_db::entity::{device, device_group, user};
use deskhaven_db::migration::Migrator;
use deskhaven_db::testing::{TestDb, each_backend, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait};
use sea_orm_migration::prelude::*;

const FK_MIGRATION: &str = "m20261102_000021_devices_foreign_keys";

fn device_with(rustdesk_id: &str, owner: Option<i64>, group: Option<i64>) -> device::ActiveModel {
    device::ActiveModel {
        owner_user_id: Set(owner),
        device_group_id: Set(group),
        ..device_row(rustdesk_id)
    }
}

/// Applies the next `steps` pending migrations (all when `None`) in one write transaction,
/// like the startup runner does (DDL statements must share one connection).
async fn migrate_steps(conn: &DatabaseConnection, steps: Option<usize>) {
    let steps = steps.map(|n| n.try_into().unwrap());
    let txn = begin_write(conn).await.unwrap();
    Migrator::up(&txn, steps).await.unwrap();
    txn.commit().await.unwrap();
}

#[tokio::test]
async fn table_and_indexes_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let manager = SchemaManager::new(&db.conn);
        assert!(manager.has_table("device_groups").await.unwrap());
        assert_indexes(
            db,
            &[
                ("device_groups", "uq_device_groups_name"),
                ("devices", "ux_devices_rustdesk_id"),
                ("devices", "idx_devices_owner_user_id"),
                ("devices", "idx_devices_device_group_id"),
            ],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn device_group_name_is_unique() {
    each_migrated_backend(async |db: &TestDb| {
        device_group_row("lab").insert(&db.conn).await.unwrap();

        let dup = device_group_row("lab").insert(&db.conn).await;

        assert!(is_unique_violation(dup), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn device_group_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = device_group_row("lab").insert(&db.conn).await.unwrap();

        let found = device_group::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap();

        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn managed_by_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        device_group_row("lab").insert(&db.conn).await.unwrap();

        assert_checks(
            db,
            &[(
                "device_groups",
                "managed_by = 'both'",
                "managed_by = 'config'",
            )],
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn devices_reject_dangling_owner_and_group() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;

        let owner = device_with("1", Some(404), None).insert(conn).await;
        let group = device_with("2", None, Some(404)).insert(conn).await;

        assert!(is_fk_violation(owner), "{}: dangling owner", db.backend);
        assert!(is_fk_violation(group), "{}: dangling group", db.backend);
        assert_eq!(count::<device::Entity>(conn).await, 0);
    })
    .await;
}

#[tokio::test]
async fn devices_accept_existing_owner_group_or_neither() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let lab = device_group_row("lab").insert(conn).await.unwrap();

        let both = device_with("1", Some(alice.id), Some(lab.id))
            .insert(conn)
            .await;
        let neither = device_with("2", None, None).insert(conn).await;

        assert_eq!(
            both.unwrap().owner_user_id,
            Some(alice.id),
            "{}",
            db.backend
        );
        assert!(neither.is_ok(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_unassigns_its_devices() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let lab = device_group_row("lab").insert(conn).await.unwrap();
        let other = device_group_row("other").insert(conn).await.unwrap();
        device_with("1", None, Some(lab.id))
            .insert(conn)
            .await
            .unwrap();
        device_with("2", None, Some(other.id))
            .insert(conn)
            .await
            .unwrap();

        device_group::Entity::delete_by_id(lab.id)
            .exec(conn)
            .await
            .unwrap();

        let mut groups: Vec<_> = device::Entity::find()
            .all(conn)
            .await
            .unwrap()
            .into_iter()
            .map(|d| (d.rustdesk_id, d.device_group_id))
            .collect();
        groups.sort();
        let b = db.backend;
        assert_eq!(
            groups,
            vec![("1".to_owned(), None), ("2".to_owned(), Some(other.id))],
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_unassigns_their_devices() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        device_with("1", Some(alice.id), None)
            .insert(conn)
            .await
            .unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let kept = device::Entity::find().all(conn).await.unwrap();
        assert_eq!(kept.len(), 1, "{}: device kept", db.backend);
        assert_eq!(kept[0].owner_user_id, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn existing_device_checks_survive_the_rebuild() {
    each_migrated_backend(async |db: &TestDb| {
        device_row("1").insert(&db.conn).await.unwrap();

        let dup = device_row("1").insert(&db.conn).await;

        assert!(is_unique_violation(dup), "{}: rustdesk_id", db.backend);
        assert_checks(db, &[("devices", "status = 'gone'", "status = 'disabled'")]).await;
    })
    .await;
}

/// Upgrading a database that already holds devices: rows survive the SQLite table rebuild
/// unchanged, and references that could never have been valid (the tables did not exist
/// yet) are cleared instead of failing the whole upgrade.
#[tokio::test]
async fn upgrade_keeps_devices_and_clears_dangling_references() {
    each_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let before_fk = Migrator::migrations()
            .iter()
            .position(|m| m.name() == FK_MIGRATION)
            .unwrap();
        migrate_steps(conn, Some(before_fk)).await;
        let dangling = device_with("1", Some(42), Some(7))
            .insert(conn)
            .await
            .unwrap();
        let plain = device_with("2", None, None).insert(conn).await.unwrap();

        migrate_steps(conn, None).await;

        let b = db.backend;
        let migrated = device::Entity::find().all(conn).await.unwrap();
        assert_eq!(migrated.len(), 2, "{b}");
        let cleared = device::Model {
            owner_user_id: None,
            device_group_id: None,
            ..dangling
        };
        assert!(migrated.contains(&cleared), "{b}: dangling refs cleared");
        assert!(migrated.contains(&plain), "{b}: plain row unchanged");
        let alice = user_row("alice").insert(conn).await.unwrap();
        let after = device_with("3", Some(alice.id), None).insert(conn).await;
        assert!(after.is_ok(), "{b}");
        let dangling_now = device_with("4", Some(99), None).insert(conn).await;
        assert!(
            is_fk_violation(dangling_now),
            "{b}: FK active after upgrade"
        );
    })
    .await;
}
