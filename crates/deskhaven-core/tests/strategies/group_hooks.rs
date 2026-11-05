//! Group commit hooks run in a task the caller cannot cancel, and only for real changes.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::task::{Context, Waker};
use std::time::Duration;

use deskhaven_core::groups::{GroupChange, GroupError};
use deskhaven_core::strategies::Target;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::timeout;

use super::support::{Ctx, ctx, device_inputs, opts};

const WAIT: Duration = Duration::from_secs(10);

/// Records every [`GroupChange`] the group service reports.
fn record(c: &Ctx) -> UnboundedReceiver<GroupChange> {
    let (tx, rx) = unbounded_channel();
    c.groups.register_commit_hook(Arc::new(move |change| {
        let _ = tx.send(*change);
    }));
    rx
}

/// Polls the future once, then drops it: what a cancelled request does.
async fn drop_after_first_poll<T>(fut: impl Future<Output = T>) {
    let mut fut = std::pin::pin!(fut);
    let _ = fut.as_mut().poll(&mut Context::from_waker(Waker::noop()));
}

async fn next(rx: &mut UnboundedReceiver<GroupChange>) -> GroupChange {
    timeout(WAIT, rx.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn hooks_still_run_when_the_caller_is_dropped_before_the_commit_returns() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let mut rx = record(&c);
        let (alice, ops) = (c.user("alice").await, c.group("ops").await);

        drop_after_first_poll(c.groups.add_member(ops.id, alice.id)).await;
        let added = GroupChange::MembershipChanged {
            group_id: ops.id,
            user_id: alice.id,
        };
        assert_eq!(next(&mut rx).await, added);
        assert_eq!(c.groups.members(ops.id).await.unwrap().len(), 1);

        drop_after_first_poll(c.groups.remove_member(ops.id, alice.id)).await;
        assert_eq!(next(&mut rx).await, added);
        assert!(c.groups.members(ops.id).await.unwrap().is_empty());

        drop_after_first_poll(c.groups.delete(ops.id)).await;
        assert_eq!(
            next(&mut rx).await,
            GroupChange::Deleted { group_id: ops.id }
        );
        assert!(matches!(
            c.groups.get(ops.id).await,
            Err(GroupError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn a_dropped_caller_still_invalidates_the_strategy_cache() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (alice, ops) = (c.user("alice").await, c.group("ops").await);
        let s = c.strategy("s", &[("k", "ops")]).await;
        c.assign(Target::Group(ops.id), &s).await;
        let inputs = device_inputs(1, Some(&alice), None);
        assert!(c.options(&inputs).await.is_empty());
        assert!(c.svc.cache().is_loaded());

        drop_after_first_poll(c.groups.add_member(ops.id, alice.id)).await;

        timeout(WAIT, async {
            while c.svc.cache().is_loaded() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(c.options(&inputs).await, opts(&[("k", "ops")]));
    })
    .await;
}

#[tokio::test]
async fn membership_hooks_fire_only_when_a_row_changed() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let mut rx = record(&c);
        let (alice, ops) = (c.user("alice").await, c.group("ops").await);
        let changed = GroupChange::MembershipChanged {
            group_id: ops.id,
            user_id: alice.id,
        };

        c.groups.add_member(ops.id, alice.id).await.unwrap();
        assert_eq!(next(&mut rx).await, changed);
        // Already a member / not a member: nothing changed, nothing to invalidate.
        c.groups.add_member(ops.id, alice.id).await.unwrap();
        c.groups.remove_member(ops.id, 9999).await.unwrap();
        assert!(c.groups.add_member(ops.id, 9999).await.is_err());
        assert!(c.groups.add_member(9999, alice.id).await.is_err());
        assert!(rx.try_recv().is_err());

        c.groups.remove_member(ops.id, alice.id).await.unwrap();
        assert_eq!(next(&mut rx).await, changed);
        c.groups.remove_member(ops.id, alice.id).await.unwrap();
        assert!(rx.try_recv().is_err());
    })
    .await;
}
