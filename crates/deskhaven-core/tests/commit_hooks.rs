//! Commit hooks of the user and group services: a panicking hook neither turns a committed
//! write into an error nor stops the later hooks or the event, and a hook may register
//! another hook without deadlocking. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deskhaven_config::settings::PinSet;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{GroupChange, GroupService, NewGroup};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, Status, UserChange, UserEvent,
    UserService,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

async fn users(db: &TestDb) -> UserService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    UserService::new(
        db.conn.clone(),
        settings,
        PasswordHasher::new(FAST).unwrap(),
    )
}

fn local(username: &str) -> NewUser {
    let password = Password::new("tundra-Lamp-42");
    NewUser::new(username, Credential::Local { password })
}

#[tokio::test]
async fn a_panicking_user_hook_does_not_abort_the_write_the_later_hooks_or_the_event() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = users(db).await;
        let alice = svc.create(local("alice")).await.unwrap().id;
        let mut events = svc.subscribe();
        svc.register_commit_hook(Arc::new(|_: &UserChange| panic!("hook bug")));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        svc.register_commit_hook(Arc::new(move |change: &UserChange| {
            sink.lock().unwrap().push(*change);
        }));

        let result = timeout(WAIT, svc.set_status(alice, Status::Disabled))
            .await
            .unwrap();

        assert!(result.is_ok(), "{result:?}");
        let disabled = UserEvent::Disabled { user_id: alice };
        assert_eq!(*seen.lock().unwrap(), [UserChange::Event(disabled)]);
        assert_eq!(events.try_recv().unwrap(), disabled);
        assert_eq!(svc.get(alice).await.unwrap().status, Status::Disabled);
    })
    .await;
}

/// Re-review LOW-5: a cache whose targeted invalidation panicked must fail closed: the hook's
/// fallback (e.g. flush the whole cache) runs instead, so a disabled user's cached session
/// cannot outlive the change.
#[tokio::test]
async fn a_panicking_user_hook_runs_its_fallback() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = users(db).await;
        let alice = svc.create(local("alice")).await.unwrap().id;
        let flushed = Arc::new(AtomicUsize::new(0));
        let calm = Arc::new(AtomicUsize::new(0));
        let (on_panic, calm_fallback) = (Arc::clone(&flushed), Arc::clone(&calm));
        svc.register_commit_hook_with_fallback(
            Arc::new(|_: &UserChange| panic!("hook bug")),
            Arc::new(move || {
                on_panic.fetch_add(1, Ordering::SeqCst);
            }),
        );
        svc.register_commit_hook_with_fallback(
            Arc::new(|_: &UserChange| {}),
            Arc::new(move || {
                calm_fallback.fetch_add(1, Ordering::SeqCst);
            }),
        );

        timeout(WAIT, svc.set_status(alice, Status::Disabled))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(flushed.load(Ordering::SeqCst), 1);
        assert_eq!(calm.load(Ordering::SeqCst), 0, "only for a panic");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_user_hook_may_register_another_hook_which_runs_from_the_next_change_on() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = users(db).await;
        let alice = svc.create(local("alice")).await.unwrap().id;
        let inner_calls = Arc::new(AtomicUsize::new(0));
        let registered = Arc::new(AtomicUsize::new(0));
        let (handle, calls, once) = (svc.clone(), Arc::clone(&inner_calls), registered.clone());
        svc.register_commit_hook(Arc::new(move |_: &UserChange| {
            if once.fetch_add(1, Ordering::SeqCst) == 0 {
                let calls = Arc::clone(&calls);
                handle.register_commit_hook(Arc::new(move |_: &UserChange| {
                    calls.fetch_add(1, Ordering::SeqCst);
                }));
            }
        }));

        timeout(WAIT, svc.set_status(alice, Status::Unverified))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inner_calls.load(Ordering::SeqCst), 0);
        timeout(WAIT, svc.set_status(alice, Status::Active))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(inner_calls.load(Ordering::SeqCst), 1);
    })
    .await;
}

#[tokio::test]
async fn a_panicking_group_hook_does_not_abort_the_write_or_the_later_hooks() {
    each_migrated_backend(async |db: &TestDb| {
        let groups = GroupService::new(db.conn.clone());
        let ops = groups.create(NewGroup::new("ops")).await.unwrap();
        groups.register_commit_hook(Arc::new(|_: &GroupChange| panic!("hook bug")));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        groups.register_commit_hook(Arc::new(move |change: &GroupChange| {
            sink.lock().unwrap().push(*change);
        }));

        let result = timeout(WAIT, groups.delete(ops.id)).await.unwrap();

        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            *seen.lock().unwrap(),
            [GroupChange::Deleted { group_id: ops.id }]
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_group_hook_may_register_another_hook_without_deadlocking() {
    each_migrated_backend(async |db: &TestDb| {
        let groups = GroupService::new(db.conn.clone());
        let ops = groups.create(NewGroup::new("ops")).await.unwrap();
        let handle = groups.clone();
        groups.register_commit_hook(Arc::new(move |_: &GroupChange| {
            handle.register_commit_hook(Arc::new(|_: &GroupChange| {}));
        }));

        let result = timeout(WAIT, groups.delete(ops.id)).await.unwrap();

        assert!(result.is_ok(), "{result:?}");
    })
    .await;
}
