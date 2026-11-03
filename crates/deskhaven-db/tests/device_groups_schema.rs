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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect,
};
use sea_orm_migration::prelude::*;

const FK_MIGRATION: &str = "m20261102_000021_devices_foreign_keys";

fn device_with(rustdesk_id: &str, owner: Option<i64>, group: Option<i64>) -> device::ActiveModel {
    device::ActiveModel {
        owner_user_id: Set(owner),
        device_group_id: Set(group),
        ..device_row(rustdesk_id)
    }
}

/// Inserts `row` into a schema older than the entity (no `RETURNING` of columns a later
/// migration adds); returns its `devices.id`.
async fn insert_old(conn: &DatabaseConnection, row: device::ActiveModel) -> i64 {
    let rustdesk_id = row.rustdesk_id.clone().unwrap();
    device::Entity::insert(row)
        .exec_without_returning(conn)
        .await
        .unwrap();
    device::Entity::find()
        .select_only()
        .column(device::Column::Id)
        .filter(device::Column::RustdeskId.eq(rustdesk_id))
        .into_tuple::<i64>()
        .one(conn)
        .await
        .unwrap()
        .unwrap()
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

/// Other tables hold `devices.id` without a foreign key, so an id that was handed out once
/// must never be handed out again, also across the SQLite table rebuild.
#[tokio::test]
async fn upgrade_does_not_reuse_deleted_device_ids() {
    each_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let before_fk = Migrator::migrations()
            .iter()
            .position(|m| m.name() == FK_MIGRATION)
            .unwrap();
        migrate_steps(conn, Some(before_fk)).await;
        insert_old(conn, device_with("1", None, None)).await;
        let highest = insert_old(conn, device_with("2", None, None)).await;
        device::Entity::delete_by_id(highest)
            .exec(conn)
            .await
            .unwrap();

        migrate_steps(conn, None).await;

        let next = device_with("3", None, None).insert(conn).await.unwrap();
        assert!(next.id > highest, "{}: id reused", db.backend);
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
        let dangling = insert_old(conn, device_with("1", Some(42), Some(7))).await;
        let plain = insert_old(conn, device_with("2", None, None)).await;
        let template = device_with("0", None, None);

        migrate_steps(conn, None).await;

        let b = db.backend;
        let mut migrated = device::Entity::find().all(conn).await.unwrap();
        migrated.sort_by_key(|row| row.id);
        assert_eq!(migrated.len(), 2, "{b}");
        // Every column as inserted, the references cleared, the columns added later empty.
        for (row, id, rustdesk_id) in [(&migrated[0], dangling, "1"), (&migrated[1], plain, "2")] {
            let seen = (row.id, row.rustdesk_id.as_str(), &row.uuid, &row.pk);
            assert_eq!(seen.0, id, "{b}: ids kept in order");
            assert_eq!(seen.1, rustdesk_id, "{b}");
            assert_eq!(
                (seen.2, seen.3),
                (template.uuid.as_ref(), template.pk.as_ref()),
                "{b}"
            );
            assert_eq!(row.register_at, *template.register_at.as_ref(), "{b}");
            let refs = (row.owner_user_id, row.device_group_id);
            assert_eq!(refs, (None, None), "{b}: dangling refs cleared");
            let names = (&row.display_name, &row.display_username);
            assert_eq!(names, (&None, &None), "{b}");
        }
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
