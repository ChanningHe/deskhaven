//! The console's device operations (M3-05): guarded writes roll back with their guard, the
//! online filter pages through presence, config-managed device groups are read-only.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};

use deskhaven_core::console_audit::{Actor, ConsoleAuditLog, NewConsoleAudit, Refused, TxGuard};
use deskhaven_core::devices::{
    DeviceError, DeviceGroupUpdate, DeviceQuery, DeviceRecord, DeviceSummary, DeviceUpdate,
};
use deskhaven_core::pagination::{Page, PageRequest};
use deskhaven_db::entity::device_group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait};

use super::support::{Ctx, ID, ctx};

const NOT_ALLOWED: &str = "not_allowed";

fn refusing() -> Refused {
    Refused::new(NOT_ALLOWED)
}

fn audit_entry(action: &str) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: Actor::System,
        action: action.to_owned(),
        target_type: "device".to_owned(),
        target_id: String::new(),
        before: None,
        after: None,
        ip: None,
    }
}

fn log(c: &Ctx) -> ConsoleAuditLog {
    ConsoleAuditLog::new(c.conn.clone(), c.clock.clone())
}

fn ids(page: Page<DeviceSummary>) -> Vec<String> {
    let records = page.items.into_iter().map(|s| s.record);
    records.map(|r| r.rustdesk_id).collect()
}

async fn audit_rows(c: &Ctx) -> u64 {
    deskhaven_db::entity::console_audit::Entity::find()
        .count(&c.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_refusing_guard_rolls_back_the_update_and_the_delete() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let id = c.device(ID).await;
        let update = DeviceUpdate {
            note: Some("changed".to_owned()),
            ..DeviceUpdate::default()
        };
        let no = |_: Option<&DeviceRecord>, _: Option<&DeviceRecord>| Err(refusing());

        // Act
        let updated = c
            .devices
            .update_guarded(id, update, TxGuard::new(&log(&c), no))
            .await;
        let deleted = c
            .devices
            .delete_guarded(id, TxGuard::new(&log(&c), no))
            .await;

        // Assert
        let b = db.backend;
        assert!(
            matches!(updated, Err(DeviceError::Refused(r)) if r.code == NOT_ALLOWED),
            "{b}"
        );
        assert!(matches!(deleted, Err(DeviceError::Refused(_))), "{b}");
        assert_eq!(c.row(ID).await.note, None, "{b}");
        assert_eq!(audit_rows(&c).await, 0, "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_accepting_guard_commits_the_change_with_its_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let id = c.device(ID).await;
        let alice = c.user("alice").await.id;
        let update = DeviceUpdate {
            owner_user_id: Some(Some(alice)),
            note: Some("  ".to_owned()),
            ..DeviceUpdate::default()
        };
        let yes = |before: Option<&DeviceRecord>, after: Option<&DeviceRecord>| {
            assert!(before.is_some() && after.is_some());
            Ok(audit_entry("device.update"))
        };

        let record = c
            .devices
            .update_guarded(id, update, TxGuard::new(&log(&c), yes))
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(record.owner_user_id, Some(alice), "{b}");
        assert_eq!(record.note, None, "a blank note clears it: {b}");
        assert_eq!(audit_rows(&c).await, 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_online_filter_pages_through_presence_in_order() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for id in ["100", "200", "300", "400", "500"] {
            c.device(id).await;
        }
        for id in ["200", "300", "500"] {
            c.devices.presence().signal_online(id, ip);
        }
        let query = |online, page| DeviceQuery {
            online: Some(online),
            page: PageRequest { page, page_size: 2 },
            ..DeviceQuery::default()
        };
        let first = c.devices.page(&query(true, 1)).await.unwrap();
        let second = c.devices.page(&query(true, 2)).await.unwrap();
        let beyond = c.devices.page(&query(true, 3)).await.unwrap();
        let offline = c.devices.page(&query(false, 1)).await.unwrap();

        let b = db.backend;
        assert_eq!(first.total, 3, "{b}");
        assert_eq!(ids(first), ["200", "300"], "{b}");
        assert_eq!(ids(second), ["500"], "{b}");
        assert!(beyond.items.is_empty() && beyond.total == 3, "{b}");
        assert_eq!(ids(offline), ["100", "400"], "{b}");
    })
    .await;
}

#[tokio::test]
async fn config_managed_device_groups_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let now = time::now();
        let row = device_group::ActiveModel {
            name: Set("fleet".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let id = row.insert(&c.conn).await.unwrap().id;
        let update = DeviceGroupUpdate {
            note: Some("x".to_owned()),
            ..DeviceGroupUpdate::default()
        };

        let updated = c
            .devices
            .update_group_guarded(
                id,
                update,
                TxGuard::new(&log(&c), |_, _| Ok(audit_entry("device_group.update"))),
            )
            .await;
        let deleted = c.devices.delete_group(id).await;

        let b = db.backend;
        assert!(matches!(updated, Err(DeviceError::ManagedByConfig)), "{b}");
        assert!(matches!(deleted, Err(DeviceError::ManagedByConfig)), "{b}");
        assert_eq!(
            c.devices.group_summary(id).await.unwrap().group.note,
            "",
            "{b}"
        );
    })
    .await;
}
