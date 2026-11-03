//! Audit retention: per-type cutoffs from `audit.retention.*`, `0` keeps forever, retired
//! connection rows are ordinary rows, a record at the cutoff stays, a failing type does not
//! stop the others, and every deleting batch appends its own `console_audit` row (`system`).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::audit::RETIRED_PREFIX;
use deskhaven_core::jobs::{AuditKind, AuditRetention, Purge, RETENTION_ACTION, RETENTION_TARGET};
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::{alarm, connection_session, console_audit, file_transfer, login_event};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ConnectionTrait, EntityTrait};
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

fn after(row: &console_audit::Model) -> Value {
    serde_json::from_str(row.after.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn each_deleting_batch_appends_its_own_system_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: an expired alarm, and five expired login events (batches of 2, 2, 1).
        let c = ctx(db).await;
        c.alarm(ago(400 * DAY)).await;
        for _ in 0..5 {
            c.login(ago(181 * DAY)).await;
        }

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        let rows = c.audit_rows(RETENTION_ACTION).await;
        let login_cutoff = T0_MS - i64::try_from((180 * DAY).as_millis()).unwrap();
        let alarm_cutoff = T0_MS - i64::try_from((365 * DAY).as_millis()).unwrap();
        let expected = [
            json!({"kind": "alarm", "deleted": 1, "cutoff": alarm_cutoff, "batch": 1}),
            json!({"kind": "login", "deleted": 2, "cutoff": login_cutoff, "batch": 2}),
            json!({"kind": "login", "deleted": 2, "cutoff": login_cutoff, "batch": 3}),
            json!({"kind": "login", "deleted": 1, "cutoff": login_cutoff, "batch": 4}),
        ];
        let got: Vec<Value> = rows.iter().map(after).collect();
        assert_eq!(got, expected, "{b}");
        for row in &rows {
            assert_eq!(row.actor_kind, ActorKind::System, "{b}");
            assert_eq!(row.actor_id, None, "{b}");
            assert_eq!(row.target_type, RETENTION_TARGET, "{b}");
            assert_eq!(row.target_id, report.run_id, "{b}: the run's id");
            assert_eq!(row.created_at.timestamp_millis(), T0_MS, "{b}");
        }
        assert_eq!(report.deleted(AuditKind::Login), 5, "{b}");
    })
    .await;
}

#[tokio::test]
async fn runs_have_distinct_ids() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let job = retention(&c);

        // Act
        let first = job.run().await;
        let second = job.run().await;

        // Assert
        assert_ne!(first.run_id, second.run_id, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_failing_kind_does_not_stop_the_others() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: the alarms table is gone; login events after it still expire.
        let c = ctx(db).await;
        c.login(ago(181 * DAY)).await;
        c.conn
            .execute_unprepared("DROP TABLE alarms")
            .await
            .unwrap();

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.failed, vec![AuditKind::Alarm], "{b}");
        assert_eq!(report.deleted(AuditKind::Login), 1, "{b}");
        assert_eq!(c.count(login_event::Entity).await, 0, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_record_exactly_at_the_cutoff_is_kept() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.login(ago(180 * DAY)).await;
        c.login(ago(180 * DAY + Duration::from_millis(1))).await;

        // Act
        let report = retention(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.deleted(AuditKind::Login), 1, "{b}");
        assert_eq!(
            c.count(login_event::Entity).await,
            1,
            "{b}: the one at the cutoff"
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
