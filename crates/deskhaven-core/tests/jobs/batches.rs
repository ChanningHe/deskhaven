//! Batched deletes: each batch commits on its own, other writers get in between two
//! batches, and a run stopped between batches keeps its audit row in step with what it
//! deleted.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::time::Duration;

use deskhaven_core::jobs::{AuditRetention, Purge, RETENTION_ACTION};
use deskhaven_db::entity::login_event;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
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

#[tokio::test]
async fn a_run_stopped_between_batches_has_counted_what_it_deleted() {
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
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        assert_eq!(after["deleted"]["login"], json!(BATCH), "{b}: {after}");
    })
    .await;
}

#[tokio::test]
async fn later_batches_update_the_same_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let run = start(&c).await;

        // Act
        wait_for(&c, 3).await;
        let midway = c.audit_rows(RETENTION_ACTION).await;
        run.await.unwrap();
        let done = c.audit_rows(RETENTION_ACTION).await;

        // Assert
        let b = db.backend;
        let count = |rows: &[deskhaven_db::entity::console_audit::Model]| -> Value {
            let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
            after["deleted"]["login"].clone()
        };
        assert_eq!((midway.len(), done.len()), (1, 1), "{b}");
        assert_eq!(midway[0].id, done[0].id, "{b}");
        assert_eq!(count(&midway), json!(6), "{b}");
        assert_eq!(count(&done), json!(9), "{b}");
    })
    .await;
}
