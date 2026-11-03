//! Batched deletes: each batch commits on its own, other writers get in between two
//! batches, each batch appends its own audit row (none is rewritten), a run stopped between
//! batches has recorded what it deleted, and a row locked elsewhere (PostgreSQL) is skipped.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::Duration;

use deskhaven_core::jobs::{AuditKind, AuditRetention, Purge, RETENTION_ACTION};
use deskhaven_db::entity::{console_audit, login_event};
use deskhaven_db::testing::{Backend, TestDb, each_migrated_backend};
use sea_orm::{ConnectionTrait, EntityTrait, QueryOrder, TransactionTrait};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::support::{Ctx, DAY, ago, ctx};

const BATCH: u64 = 3;
/// Long enough for the test to observe (and write) between two batches.
const PAUSE: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(5);
const PATIENCE: Duration = Duration::from_secs(10);

/// Nine expired login events, then a retention run in the background.
async fn start(c: &Ctx) -> JoinHandle<()> {
    for _ in 0..9 {
        c.login(ago(200 * DAY)).await;
    }
    let s = &c.state;
    let job = AuditRetention::new(s.db.clone(), s.settings.clone(), s.clock.clone())
        .with_purge(Purge::new(BATCH, PAUSE));
    tokio::spawn(async move {
        job.run().await;
    })
}

async fn logins(c: &Ctx) -> u64 {
    c.count(login_event::Entity).await
}

/// Waits until `logins` reads `n`.
async fn wait_for(c: &Ctx, n: u64) {
    tokio::time::timeout(PATIENCE, async {
        while logins(c).await != n {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("never saw {n} login events"));
}

#[tokio::test]
async fn every_batch_commits_on_its_own() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let run = start(&c).await;

        // Act: watch the table from another connection while the run goes on.
        let mut seen = BTreeSet::new();
        while !run.is_finished() {
            seen.insert(logins(&c).await);
            tokio::time::sleep(POLL).await;
        }
        run.await.unwrap();
        seen.insert(logins(&c).await);

        // Assert: only whole batches ever became visible, one at a time.
        let b = db.backend;
        assert!(
            seen.is_subset(&BTreeSet::from([9, 6, 3, 0])),
            "{b}: {seen:?}"
        );
        assert!(seen.contains(&6) && seen.contains(&3), "{b}: {seen:?}");
        assert!(seen.contains(&0), "{b}: {seen:?}");
    })
    .await;
}

#[tokio::test]
async fn another_writer_commits_between_two_batches() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let run = start(&c).await;
        wait_for(&c, 6).await;

        // Act: a write (a fresh login event) while the run still has batches to go.
        c.login(ago(DAY)).await;
        let wrote_during_run = !run.is_finished();
        run.await.unwrap();

        // Assert
        let b = db.backend;
        assert!(wrote_during_run, "{b}: the write waited for the whole run");
        assert_eq!(logins(&c).await, 1, "{b}: only the fresh event is left");
    })
    .await;
}

fn after(row: &console_audit::Model) -> Value {
    serde_json::from_str(row.after.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn a_run_stopped_between_batches_has_recorded_what_it_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let run = start(&c).await;
        wait_for(&c, 6).await;

        // Act: shutdown drops the run during its pause.
        run.abort();
        let _ = run.await;

        // Assert
        let b = db.backend;
        assert_eq!(logins(&c).await, 6, "{b}: one batch deleted");
        let rows = c.audit_rows(RETENTION_ACTION).await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(after(&rows[0])["deleted"], json!(BATCH), "{b}");
    })
    .await;
}

#[tokio::test]
async fn every_batch_appends_a_row_and_none_is_rewritten() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let run = start(&c).await;

        // Act
        wait_for(&c, 3).await;
        let midway = c.audit_rows(RETENTION_ACTION).await;
        run.await.unwrap();
        let done = c.audit_rows(RETENTION_ACTION).await;

        // Assert: the rows seen midway are unchanged at the end, one more was appended.
        let b = db.backend;
        assert_eq!((midway.len(), done.len()), (2, 3), "{b}");
        assert_eq!(midway[..], done[..2], "{b}");
        let batches: Vec<Value> = done.iter().map(|r| after(r)["batch"].clone()).collect();
        assert_eq!(batches, [json!(1), json!(2), json!(3)], "{b}");
        assert!(done.iter().all(|r| r.target_id == done[0].target_id), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_row_locked_by_another_transaction_is_skipped_not_waited_for() {
    each_migrated_backend(async |db: &TestDb| {
        if db.backend != Backend::Postgres {
            return; // SQLite has no row locks.
        }
        // Arrange: three expired login events, one of them locked by another transaction.
        let c = ctx(db).await;
        for _ in 0..3 {
            c.login(ago(200 * DAY)).await;
        }
        let locked = login_event::Entity::find()
            .order_by_asc(login_event::Column::Id)
            .one(&c.conn)
            .await
            .unwrap()
            .unwrap()
            .id;
        let holder = c.conn.begin().await.unwrap();
        let sql = format!("SELECT id FROM login_events WHERE id = {locked} FOR UPDATE");
        holder.execute_unprepared(&sql).await.unwrap();
        let s = &c.state;
        let job = AuditRetention::new(s.db.clone(), s.settings.clone(), s.clock.clone())
            .with_purge(Purge::new(BATCH, Duration::ZERO));

        // Act
        let report = tokio::time::timeout(PATIENCE, job.run()).await;
        holder.rollback().await.unwrap();

        // Assert
        let report = report.expect("the purge waited for the locked row");
        assert_eq!(report.deleted(AuditKind::Login), 2);
        let left = login_event::Entity::find().all(&c.conn).await.unwrap();
        let ids: Vec<i64> = left.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![locked], "the locked row is left for the next run");
    })
    .await;
}
