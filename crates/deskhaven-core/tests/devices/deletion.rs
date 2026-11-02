//! Deleting a device or device group removes its strategy assignment in the same transaction
//! and invalidates the strategy cache (`strategy_assignments.target_id` has no foreign key).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

use deskhaven_core::devices::DeviceError;
use deskhaven_core::strategies::Target;
use deskhaven_db::entity::strategy_assignment;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

use super::support::{ID, ctx};

async fn assignment_rows(db: &TestDb) -> u64 {
    strategy_assignment::Entity::find()
        .count(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_a_device_removes_it_its_assignment_and_its_presence() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let other = c.device("987654321").await;
        let s = c.strategy("s").await.id;
        c.strategies
            .assign(Target::Device(device_id), s)
            .await
            .unwrap();
        c.strategies.assign(Target::Device(other), s).await.unwrap();
        c.strategies.effective(&Default::default()).await.unwrap();
        let presence = c.devices.presence();
        presence.signal_online(ID, IpAddr::V4(Ipv4Addr::LOCALHOST));

        // Act
        let deleted = c.devices.delete(device_id).await.unwrap();
        let again = c.devices.delete(device_id).await.unwrap();

        // Assert
        let b = db.backend;
        assert!(deleted && !again, "{b}");
        let gone = deskhaven_core::devices::DeviceRepository::new(c.conn.clone())
            .find_by_rustdesk_id(ID)
            .await
            .unwrap();
        assert!(gone.is_none(), "{b}");
        let assigned = c.strategies.assignment(Target::Device(device_id)).await;
        assert_eq!(assigned.unwrap(), None, "{b}");
        assert_eq!(
            assignment_rows(db).await,
            1,
            "{b}: the other device keeps its own"
        );
        assert!(!c.strategies.cache().is_loaded(), "{b}: cache invalidated");
        assert_eq!(presence.get(ID), None, "{b}");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_device_group_removes_its_assignment_and_ungroups_its_devices() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let group = c.devices.create_group("Office", "").await.unwrap();
        let s = c.strategy("s").await.id;
        c.strategies
            .assign(Target::DeviceGroup(group), s)
            .await
            .unwrap();
        set_group(&c, group).await;
        c.strategies.effective(&Default::default()).await.unwrap();

        // Act
        let deleted = c.devices.delete_group(group).await.unwrap();
        let again = c.devices.delete_group(group).await.unwrap();

        // Assert
        let b = db.backend;
        assert!(deleted && !again, "{b}");
        assert_eq!(assignment_rows(db).await, 0, "{b}");
        assert_eq!(c.row(ID).await.device_group_id, None, "{b}");
        assert!(!c.strategies.cache().is_loaded(), "{b}: cache invalidated");
    })
    .await;
}

async fn set_group(c: &super::support::Ctx, group: i64) {
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, IntoActiveModel};
    let mut row = c.row(ID).await.into_active_model();
    row.device_group_id = Set(Some(group));
    row.update(&c.conn).await.unwrap();
}

#[tokio::test]
async fn device_group_names_are_validated_and_unique() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.devices.create_group(" Office ", "").await.unwrap();

        // Act
        let duplicate = c.devices.create_group("Office", "").await;
        let blank = c.devices.create_group("  ", "").await;
        let control = c.devices.create_group("a\u{7}b", "").await;

        // Assert
        let b = db.backend;
        assert!(
            matches!(duplicate, Err(DeviceError::Conflict { field: "name" })),
            "{b}: {duplicate:?}"
        );
        assert!(matches!(blank, Err(DeviceError::Invalid { .. })), "{b}");
        assert!(matches!(control, Err(DeviceError::Invalid { .. })), "{b}");
    })
    .await;
}

/// Deletion hooks (the signal server's `reload_peer` in `serve`) run once per committed
/// deletion, with the RustDesk id; never for a missing device.
#[tokio::test]
async fn deletion_hooks_run_once_per_deleted_device_with_its_rustdesk_id() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let record = seen.clone();
        c.devices.on_device_deleted(Arc::new(move |id: &str| {
            record.lock().unwrap().push(id.to_owned());
        }));

        // Act
        c.devices.delete(device_id).await.unwrap();
        c.devices.delete(device_id).await.unwrap();

        // Assert
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen, [ID.to_owned()], "{}", db.backend);
    })
    .await;
}
