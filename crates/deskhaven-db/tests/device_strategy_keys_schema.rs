//! `device_strategy_keys` (M2-14): the strategy option keys last sent to a device, one row per
//! RustDesk id, independent of the `devices` row (it outlives a deletion and re-registration),
//! on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::ops::{device_row, is_unique_violation};
use common::ts;
use deskhaven_db::entity::{device, device_strategy_key};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

fn keys_row(rustdesk_id: &str, keys: &str) -> device_strategy_key::ActiveModel {
    device_strategy_key::ActiveModel {
        rustdesk_id: Set(rustdesk_id.to_owned()),
        option_keys: Set(keys.to_owned()),
        updated_at: Set(ts(1_000)),
    }
}

#[tokio::test]
async fn one_row_per_rustdesk_id_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let manager = SchemaManager::new(&db.conn);

        // Act
        let inserted = keys_row("123456789", r#"["a","b"]"#)
            .insert(&db.conn)
            .await
            .unwrap();
        let dup = keys_row("123456789", "[]").insert(&db.conn).await;
        let found = device_strategy_key::Entity::find_by_id("123456789".to_owned())
            .one(&db.conn)
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        let has_table = manager.has_table("device_strategy_keys").await.unwrap();
        assert!(has_table, "{b}");
        assert!(is_unique_violation(dup), "{b}");
        assert_eq!(found, Some(inserted), "{b}");
    })
    .await;
}

/// A deleted device's client keeps the options it was sent; the row must survive so the
/// re-registered device still gets them deleted.
#[tokio::test]
async fn rows_need_no_device_and_outlive_its_deletion() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let device = device_row("123456789").insert(&db.conn).await.unwrap();
        keys_row("123456789", r#"["k"]"#)
            .insert(&db.conn)
            .await
            .unwrap();

        // Act
        let unregistered = keys_row("555555555", "[]").insert(&db.conn).await;
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
        assert!(unregistered.is_ok(), "{b}: no foreign key");
        assert_eq!(left.len(), 2, "{b}: kept after the device was deleted");
    })
    .await;
}
