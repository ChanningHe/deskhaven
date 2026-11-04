//! The console's entry points (M3-07): guarded writes that commit with their audit row or
//! not at all, option patches, the paged list with assignment counts and the named targets
//! of a strategy.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use deskhaven_core::console_audit::{Actor, ConsoleAuditLog, NewConsoleAudit, Refused, TxGuard};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::strategies::{
    AssignedTarget, Assignment, NewStrategy, OptionsPatch, Strategy, StrategyError, StrategyQuery,
    StrategySort, StrategySortField, StrategyUpdate, Target,
};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::{console_audit, strategy};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait};

use super::support::{Ctx, ctx, device_inputs, opts};

type Seen<T> = Arc<Mutex<Vec<(Option<T>, Option<T>)>>>;

/// A guard that records what it was shown and writes a row named `action`.
fn recording<T: Clone + Send + 'static>(c: &Ctx, action: &str, seen: &Seen<T>) -> TxGuard<T> {
    let log = ConsoleAuditLog::new(c.conn.clone(), c.clock.clone());
    let (action, seen) = (action.to_owned(), Arc::clone(seen));
    TxGuard::new(&log, move |before: Option<&T>, after: Option<&T>| {
        seen.lock().unwrap().push((before.cloned(), after.cloned()));
        Ok(entry(&action))
    })
}

fn refusing<T: 'static>(c: &Ctx) -> TxGuard<T> {
    let log = ConsoleAuditLog::new(c.conn.clone(), c.clock.clone());
    TxGuard::new(&log, |_, _| Err(Refused::new("nope")))
}

fn entry(action: &str) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: Actor::Cli,
        action: action.to_owned(),
        target_type: "strategy".to_owned(),
        target_id: String::new(),
        before: None,
        after: None,
        ip: None,
    }
}

async fn audit_count(c: &Ctx) -> u64 {
    console_audit::Entity::find().count(&c.conn).await.unwrap()
}

fn seen<T>() -> Seen<T> {
    Arc::new(Mutex::new(Vec::new()))
}

#[tokio::test]
async fn guarded_crud_commits_with_its_audit_row_or_not_at_all() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let log: Seen<Strategy> = seen();

        let refused = c
            .svc
            .create_guarded(NewStrategy::new("kiosk", opts(&[])), refusing(&c))
            .await;
        assert!(matches!(refused, Err(StrategyError::Refused(r)) if r.code == "nope"));
        assert!(c.svc.list().await.unwrap().is_empty());

        let new = NewStrategy::new("kiosk", opts(&[("a", "1")]));
        let created = c
            .svc
            .create_guarded(new, recording(&c, "strategy.create", &log))
            .await
            .unwrap();
        let update = StrategyUpdate {
            note: Some("n".to_owned()),
            ..StrategyUpdate::default()
        };
        let refused = c
            .svc
            .update_guarded(created.id, update.clone(), refusing(&c));
        assert!(matches!(refused.await, Err(StrategyError::Refused(_))));
        assert_eq!(c.svc.get(created.id).await.unwrap().note, "");
        let updated = c
            .svc
            .update_guarded(created.id, update, recording(&c, "strategy.update", &log))
            .await
            .unwrap();
        let refused = c.svc.delete_guarded(created.id, refusing(&c)).await;
        assert!(matches!(refused, Err(StrategyError::Refused(_))));
        c.svc
            .delete_guarded(created.id, recording(&c, "strategy.delete", &log))
            .await
            .unwrap();

        assert_eq!(audit_count(&c).await, 3);
        let calls = log.lock().unwrap().clone();
        assert_eq!(calls[0], (None, Some(created.clone())));
        assert_eq!(calls[1], (Some(created), Some(updated.clone())));
        assert_eq!(calls[2], (Some(updated), None));
        assert!(matches!(
            c.svc.get(calls[2].0.as_ref().unwrap().id).await,
            Err(StrategyError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn an_options_patch_sets_and_removes_single_keys_and_invalidates() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let s = c
            .strategy("s", &[("keep", "1"), ("change", "1"), ("drop", "1")])
            .await;
        let device = c.device("100").await;
        c.assign(Target::Device(device), &s).await;
        let inputs = device_inputs(device, None, None);
        let before = c.svc.effective(&inputs).await.unwrap().modified_at;
        let patch = OptionsPatch::from([
            ("change".to_owned(), Some("2".to_owned())),
            ("drop".to_owned(), None),
            ("new".to_owned(), Some(String::new())),
            ("never-there".to_owned(), None),
        ]);
        let update = StrategyUpdate {
            options_patch: patch,
            ..StrategyUpdate::default()
        };

        let updated = c.svc.update(s.id, update).await.unwrap();

        let want = opts(&[("keep", "1"), ("change", "2"), ("new", "")]);
        assert_eq!(updated.options, want);
        let effective = c.svc.effective(&inputs).await.unwrap();
        assert_eq!(effective.options, want);
        assert!(effective.modified_at > before);

        let bad = StrategyUpdate {
            options_patch: OptionsPatch::from([("has space".to_owned(), Some("v".to_owned()))]),
            ..StrategyUpdate::default()
        };
        let rejected = c.svc.update(s.id, bad).await;
        assert!(matches!(
            rejected,
            Err(StrategyError::Invalid {
                field: "options",
                ..
            })
        ));
    })
    .await;
}

#[tokio::test]
async fn guarded_assignment_shows_the_replaced_one_and_unassign_checks_the_strategy() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let (a, b) = (c.strategy("a", &[]).await, c.strategy("b", &[]).await);
        let group = c.device_group("office").await;
        let target = Target::DeviceGroup(group);
        let log: Seen<Assignment> = seen();
        let at = |strategy: &Strategy| Assignment {
            target,
            strategy_id: strategy.id,
        };

        let refused = c.svc.assign_guarded(target, a.id, refusing(&c)).await;
        assert!(matches!(refused, Err(StrategyError::Refused(_))));
        assert_eq!(c.svc.assignment(target).await.unwrap(), None);
        let guard = |action| recording(&c, action, &log);
        c.svc
            .assign_guarded(target, a.id, guard("assign"))
            .await
            .unwrap();
        c.svc
            .assign_guarded(target, b.id, guard("assign"))
            .await
            .unwrap();
        let other = c
            .svc
            .unassign_guarded(target, a.id, guard("unassign"))
            .await;
        assert!(!other.unwrap(), "the target has b, not a");
        assert_eq!(c.svc.assignment(target).await.unwrap(), Some(b.id));
        assert!(
            c.svc
                .unassign_guarded(target, b.id, guard("unassign"))
                .await
                .unwrap()
        );

        let calls = log.lock().unwrap().clone();
        assert_eq!(
            calls,
            [
                (None, Some(at(&a))),
                (Some(at(&a)), Some(at(&b))),
                (Some(at(&b)), None)
            ]
        );
        assert_eq!(audit_count(&c).await, 3, "a no-op unassign records nothing");
        let missing = c
            .svc
            .assign_guarded(Target::DeviceGroup(group + 99), a.id, guard("assign"))
            .await;
        assert!(matches!(missing, Err(StrategyError::TargetNotFound(_))));
    })
    .await;
}

#[tokio::test]
async fn the_page_filters_sorts_and_counts_assignments() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let kiosk = c.strategy("Kiosk", &[("a", "1"), ("b", "2")]).await;
        let office = c.strategy("office", &[]).await;
        c.strategy("lab", &[]).await;
        let alice = c.user("alice").await;
        let staff = c.group("staff").await;
        let floor = c.device_group("floor").await;
        let device = c.device("100").await;
        for target in [
            Target::Global,
            Target::User(alice.id),
            Target::Group(staff.id),
            Target::DeviceGroup(floor),
            Target::Device(device),
        ] {
            c.assign(target, &kiosk).await;
        }

        let all = c.svc.page(&StrategyQuery::default()).await.unwrap();
        let names: Vec<_> = all.items.iter().map(|s| s.strategy.name.as_str()).collect();
        assert_eq!(names, ["Kiosk", "lab", "office"]);
        assert_eq!((all.total, all.items[0].assignment_count), (3, 5));
        let query = StrategyQuery {
            q: Some("O".to_owned()),
            sort: Some(StrategySort {
                field: StrategySortField::Name,
                descending: true,
            }),
            page: PageRequest::default(),
        };
        let found = c.svc.page(&query).await.unwrap();
        let names: Vec<_> = found
            .items
            .iter()
            .map(|s| s.strategy.name.as_str())
            .collect();
        assert_eq!(names, ["office", "Kiosk"], "case-insensitive substring");
        assert_eq!(c.svc.summary(office.id).await.unwrap().assignment_count, 0);

        let targets = c.svc.assigned_targets(kiosk.id).await.unwrap();
        let named: Vec<_> = targets
            .iter()
            .map(|t: &AssignedTarget| (t.target, t.name.clone()))
            .collect();
        assert_eq!(
            named,
            [
                (Target::Global, None),
                (Target::Group(staff.id), Some("staff".to_owned())),
                (Target::User(alice.id), Some("alice".to_owned())),
                (Target::DeviceGroup(floor), Some("floor".to_owned())),
                (Target::Device(device), Some("100".to_owned())),
            ]
        );
        assert!(c.svc.assigned_targets(office.id).await.unwrap().is_empty());
        assert!(matches!(
            c.svc.assigned_targets(office.id + 99).await,
            Err(StrategyError::NotFound)
        ));
    })
    .await;
}

#[tokio::test]
async fn concurrent_patches_of_different_options_both_survive() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let s = c.strategy("s", &[("keep", "1")]).await;
        let patch = |key: &str, value: &str| StrategyUpdate {
            options_patch: OptionsPatch::from([(key.to_owned(), Some(value.to_owned()))]),
            ..StrategyUpdate::default()
        };
        let log: Seen<Strategy> = seen();

        let (a, b) = tokio::join!(
            c.svc.update_guarded(
                s.id,
                patch("a", "x"),
                recording(&c, "strategy.update", &log)
            ),
            c.svc.update_guarded(
                s.id,
                patch("b", "y"),
                recording(&c, "strategy.update", &log)
            ),
        );

        a.unwrap();
        b.unwrap();
        let stored = c.svc.get(s.id).await.unwrap().options;
        assert_eq!(stored, opts(&[("keep", "1"), ("a", "x"), ("b", "y")]));
        assert_eq!(audit_count(&c).await, 2);
    })
    .await;
}

#[tokio::test]
async fn the_console_cannot_change_assignments_of_configuration_strategies() {
    each_migrated_backend(async |db: &TestDb| {
        let c = ctx(db).await;
        let now = time::now();
        let pinned = strategy::ActiveModel {
            name: Set("pinned".to_owned()),
            options: Set("{}".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let pinned = pinned.insert(&c.conn).await.unwrap().id;
        let ui = c.strategy("ui", &[]).await;
        let (group, other) = (c.device_group("a").await, c.device_group("b").await);
        let target = Target::DeviceGroup(group);
        let log: Seen<Assignment> = seen();
        let guard = || recording(&c, "assign", &log);

        let assigned = c.svc.assign_guarded(target, pinned, guard()).await;
        assert!(matches!(assigned, Err(StrategyError::ManagedByConfig)));
        // The configuration file (here: the unguarded path) may still assign it.
        c.svc.assign(target, pinned).await.unwrap();
        let replaced = c.svc.assign_guarded(target, ui.id, guard()).await;
        assert!(matches!(replaced, Err(StrategyError::ManagedByConfig)));
        let removed = c.svc.unassign_guarded(target, pinned, guard()).await;
        assert!(matches!(removed, Err(StrategyError::ManagedByConfig)));

        assert_eq!(c.svc.assignment(target).await.unwrap(), Some(pinned));
        let elsewhere = c
            .svc
            .assign_guarded(Target::DeviceGroup(other), ui.id, guard());
        elsewhere.await.unwrap();
        assert_eq!(
            audit_count(&c).await,
            1,
            "only the allowed assignment is recorded"
        );
    })
    .await;
}
