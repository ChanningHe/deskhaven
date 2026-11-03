//! Audit retention: per-type cutoffs from `audit.retention.*`, `0` keeps forever, retired
//! connection rows are ordinary rows, and every deleting run is recorded in
//! `console_audit` (actor `system`).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::RETIRED_PREFIX;
use deskhaven_core::jobs::{AuditKind, AuditRetention, Purge, RETENTION_ACTION};
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::{alarm, connection_session, console_audit, file_transfer, login_event};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

use super::support::{Ctx, DAY, HOUR, T0_MS, ago, ctx};

fn retention(c: &Ctx) -> AuditRetention {
    let s = &c.state;
    AuditRetention::new(s.db.clone(), s.settings.clone(), s.clock.clone())
        .with_purge(Purge::new(2, Duration::ZERO))
}

/// One row of each type just past and one just inside the default retention.
async fn seed_both_sides(c: &Ctx) {
    for (session, age) in [("1", 365 * DAY + HOUR), ("2", 365 * DAY - HOUR)] {
        c.connection(session, ago(age)).await;
        c.file(ago(age)).await;
        c.alarm(ago(age)).await;
    }
    c.login(ago(180 * DAY + HOUR)).await;
    c.login(ago(180 * DAY - HOUR)).await;
    c.console("user.create", ago(730 * DAY + HOUR)).await;
    c.console("user.create", ago(730 * DAY - HOUR)).await;
}

#[tokio::test]
async fn each_type_keeps_what_its_retention_covers() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        seed_both_sides(&c).await;

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        for kind in AuditKind::ALL {
            assert_eq!(report.deleted(kind), 1, "{b}: {kind:?}");
        }
        assert!(report.failed.is_empty(), "{b}");
        assert_eq!(c.count(connection_session::Entity).await, 1, "{b}");
        assert_eq!(c.count(file_transfer::Entity).await, 1, "{b}");
        assert_eq!(c.count(alarm::Entity).await, 1, "{b}");
        assert_eq!(c.count(login_event::Entity).await, 1, "{b}");
        let kept = c.audit_rows("user.create").await;
        assert_eq!(kept.len(), 1, "{b}");
        assert_eq!(kept[0].created_at, ago(730 * DAY - HOUR), "{b}");
    })
    .await;
}

#[tokio::test]
async fn zero_retention_keeps_records_forever() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.set("audit.retention.connection", json!("0")).await;
        c.set("audit.retention.alarm", json!("1d")).await;
        c.connection("1", ago(3000 * DAY)).await;
        c.alarm(ago(DAY + HOUR)).await;
        c.alarm(ago(DAY - HOUR)).await;

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(c.count(connection_session::Entity).await, 1, "{b}: kept");
        assert_eq!(c.count(alarm::Entity).await, 1, "{b}: 1d applies");
        let kinds: Vec<AuditKind> = report.deleted.iter().map(|(k, _)| *k).collect();
        assert!(!kinds.contains(&AuditKind::Connection), "{b}: {kinds:?}");
    })
    .await;
}

#[tokio::test]
async fn retired_connection_rows_follow_the_same_retention() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let old_retired = format!("{RETIRED_PREFIX}17");
        let new_retired = format!("{RETIRED_PREFIX}18");
        c.connection(&old_retired, ago(366 * DAY)).await;
        let kept = c.connection(&new_retired, ago(364 * DAY)).await;

        // Act
        retention(&c).run().await;

        // Assert
        let rows = connection_session::Entity::find()
            .all(&c.conn)
            .await
            .unwrap();
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![kept], "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_deleting_run_writes_one_system_audit_row_with_cutoffs_and_counts() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: five expired login events, more than two batches of two.
        let c = ctx(db).await;
        for _ in 0..5 {
            c.login(ago(181 * DAY)).await;
        }
        c.alarm(ago(400 * DAY)).await;

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        let rows = c.audit_rows(RETENTION_ACTION).await;
        assert_eq!(rows.len(), 1, "{b}: one row per run");
        let row = &rows[0];
        assert_eq!(report.audit_id, Some(row.id), "{b}");
        assert_eq!(row.actor_kind, ActorKind::System, "{b}");
        assert_eq!(row.actor_id, None, "{b}");
        assert_eq!(row.created_at.timestamp_millis(), T0_MS, "{b}");
        let after: Value = serde_json::from_str(row.after.as_deref().unwrap()).unwrap();
        assert_eq!(after["deleted"]["login"], json!(5), "{b}: {after}");
        assert_eq!(after["deleted"]["alarm"], json!(1), "{b}: {after}");
        assert_eq!(after["deleted"]["console"], json!(0), "{b}: {after}");
        let login_cutoff = T0_MS - i64::try_from((180 * DAY).as_millis()).unwrap();
        assert_eq!(
            after["cutoff"]["login"],
            json!(login_cutoff),
            "{b}: {after}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_run_that_deletes_nothing_records_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.login(ago(DAY)).await;

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.total(), 0, "{b}");
        assert_eq!(report.audit_id, None, "{b}");
        assert!(c.audit_rows(RETENTION_ACTION).await.is_empty(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_clock_decides_what_is_expired() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: a login event that expires a day from now.
        let c = ctx(db).await;
        c.login(ago(179 * DAY)).await;
        let job = retention(&c);
        let before = job.run().await;

        // Act
        c.clock.advance(DAY + HOUR);
        let after = job.run().await;

        // Assert
        let b = db.backend;
        assert_eq!(before.deleted(AuditKind::Login), 0, "{b}");
        assert_eq!(after.deleted(AuditKind::Login), 1, "{b}");
        assert_eq!(
            c.count(console_audit::Entity).await,
            1,
            "{b}: its audit row"
        );
    })
    .await;
}
