//! `device_strategy_keys` (M2-14): the strategy option keys last sent to a device, one row per
//! device, dropped with the device, on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::ops::{device_row, is_fk_violation, is_unique_violation};
use common::ts;
use deskhaven_db::entity::{device, device_strategy_key};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

fn keys_row(device_id: i64, keys: &str) -> device_strategy_key::ActiveModel {
    device_strategy_key::ActiveModel {
        device_id: Set(device_id),
        option_keys: Set(keys.to_owned()),
        updated_at: Set(ts(1_000)),
    }
}

#[tokio::test]
async fn one_row_per_device_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let manager = SchemaManager::new(&db.conn);
        let device = device_row("123456789").insert(&db.conn).await.unwrap();

        // Act
        let inserted = keys_row(device.id, r#"["a","b"]"#)
            .insert(&db.conn)
            .await
            .unwrap();
        let dup = keys_row(device.id, "[]").insert(&db.conn).await;
        let found = device_strategy_key::Entity::find_by_id(device.id)
            .one(&db.conn)
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        assert!(
            manager.has_table("device_strategy_keys").await.unwrap(),
            "{b}"
        );
        assert!(is_unique_violation(dup), "{b}");
        assert_eq!(found, Some(inserted), "{b}");
    })
    .await;
}

#[tokio::test]
async fn rows_need_a_device_and_go_with_it() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let device = device_row("123456789").insert(&db.conn).await.unwrap();
        keys_row(device.id, "[]").insert(&db.conn).await.unwrap();

        // Act
        let orphan = keys_row(device.id + 1_000, "[]").insert(&db.conn).await;
        device::Entity::delete_by_id(device.id)
            .exec(&db.conn)
            .await
            .unwrap();
        let left = device_strategy_key::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        assert!(is_fk_violation(orphan), "{b}");
        assert!(left.is_empty(), "{b}: cascaded with the device");
    })
    .await;
}
