//! Resets to the default, guarded (audited) updates and the history reader of the settings
//! service. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::Source;
use deskhaven_config::settings::{self, Change, PinSet, Reset};
use deskhaven_core::clock::SystemClock;
use deskhaven_core::console_audit::{Actor, ConsoleAuditLog, NewConsoleAudit, Refused, TxGuard};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::settings::{AppliedChanges, SettingsError, SettingsService, UpdateRequest};
use deskhaven_db::entity::{console_audit, setting, setting_history};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait, QueryOrder};
use serde_json::{Value as Json, json};

const ACTOR: i64 = 42;
const PLAINTEXT: &str = "smtp-pass-0042";

fn crypto() -> Arc<SecretBox> {
    Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])))
}

async fn service(db: &TestDb) -> SettingsService {
    let (service, warnings) = SettingsService::load(db.conn.clone(), crypto(), PinSet::empty())
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{}: {warnings:?}", db.backend);
    service
}

fn change(key: &str, value: Json, version: i64) -> Change {
    Change {
        key: key.to_owned(),
        value,
        version,
    }
}

fn reset(key: &str, version: i64) -> Reset {
    Reset {
        key: key.to_owned(),
        version,
    }
}

fn request(changes: Vec<Change>) -> UpdateRequest {
    UpdateRequest {
        changes,
        actor: Some(ACTOR),
        reason: None,
    }
}

async fn set(svc: &SettingsService, key: &str, value: Json, version: i64) {
    svc.update(request(vec![change(key, value, version)]))
        .await
        .unwrap();
}

async fn history(db: &TestDb) -> Vec<setting_history::Model> {
    setting_history::Entity::find()
        .order_by_asc(setting_history::Column::Id)
        .all(&db.conn)
        .await
        .unwrap()
}

async fn stored_rows(db: &TestDb) -> u64 {
    setting::Entity::find().count(&db.conn).await.unwrap()
}

fn audit_log(db: &TestDb) -> ConsoleAuditLog {
    ConsoleAuditLog::new(db.conn.clone(), Arc::new(SystemClock))
}

/// A guard that records one audit row listing the written keys.
fn recording_guard(db: &TestDb) -> TxGuard<AppliedChanges> {
    TxGuard::new(&audit_log(db), |_, after: Option<&AppliedChanges>| {
        let changes = after.map(|a| a.changes.clone()).unwrap_or_default();
        let shown: Vec<Json> = changes
            .iter()
            .map(
                |c| json!({ "key": c.key, "before": c.before, "after": c.after, "reset": c.reset }),
            )
            .collect();
        Ok(NewConsoleAudit {
            actor: Actor::User(ACTOR),
            action: "setting.update".to_owned(),
            target_type: "setting".to_owned(),
            target_id: String::new(),
            before: None,
            after: Some(Json::Array(shown)),
            ip: None,
        })
    })
}

async fn audit_rows(db: &TestDb) -> Vec<console_audit::Model> {
    console_audit::Entity::find().all(&db.conn).await.unwrap()
}

#[tokio::test]
async fn reset_drops_the_row_publishes_the_default_and_records_it() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "auth.session_ttl", json!("60d"), 0).await;

        let next = svc
            .update_guarded(
                request(Vec::new()),
                vec![reset("auth.session_ttl", 1)],
                recording_guard(db),
            )
            .await
            .unwrap();

        let view = next.view("auth.session_ttl").unwrap();
        assert_eq!(
            (view.value, view.source, view.version),
            (json!("30d"), Source::Default, 0),
            "{}",
            db.backend
        );
        assert_eq!(stored_rows(db).await, 0, "{}", db.backend);
        let rows = history(db).await;
        let last = rows.last().unwrap();
        assert_eq!(
            (
                last.reset,
                last.version,
                last.old_value.as_deref(),
                last.new_value.as_str()
            ),
            (true, 2, Some("\"60d\""), "\"30d\""),
            "{}",
            db.backend
        );
        assert_eq!(last.updated_by, Some(ACTOR));
        let reloaded = service(db).await.snapshot();
        assert_eq!(
            reloaded.view("auth.session_ttl").unwrap().source,
            Source::Default,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn reset_of_a_stale_version_conflicts_and_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "auth.session_ttl", json!("60d"), 0).await;
        set(&svc, "auth.session_ttl", json!("90d"), 1).await;

        let err = svc
            .update_guarded(
                request(Vec::new()),
                vec![reset("auth.session_ttl", 1)],
                recording_guard(db),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(&err, SettingsError::VersionConflict { keys } if keys == &["auth.session_ttl"]),
            "{}: {err:?}",
            db.backend
        );
        assert_eq!(stored_rows(db).await, 1, "{}", db.backend);
        assert_eq!(history(db).await.len(), 2, "{}", db.backend);
        assert!(audit_rows(db).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn resetting_a_key_without_a_row_is_a_no_op() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;

        svc.update_guarded(
            request(Vec::new()),
            vec![reset("auth.session_ttl", 0)],
            recording_guard(db),
        )
        .await
        .unwrap();

        assert!(history(db).await.is_empty(), "{}", db.backend);
        assert!(audit_rows(db).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_row_holding_the_default_value_is_still_dropped_by_a_reset() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        // Same as the default (587), but stored: the source is "modified".
        set(&svc, "smtp.port", json!(2525), 0).await;
        set(&svc, "smtp.port", json!(587), 1).await;
        assert!(matches!(
            svc.snapshot().view("smtp.port").unwrap().source,
            Source::Database { .. }
        ));

        let next = svc
            .update_guarded(
                request(Vec::new()),
                vec![reset("smtp.port", 2)],
                recording_guard(db),
            )
            .await
            .unwrap();

        assert_eq!(
            next.view("smtp.port").unwrap().source,
            Source::Default,
            "{}",
            db.backend
        );
        assert_eq!(stored_rows(db).await, 0, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_reset_that_breaks_a_cross_key_rule_rolls_the_batch_back() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        svc.update(request(vec![
            change("smtp.host", json!("mail.example.com"), 0),
            change("smtp.from", json!("noreply@example.com"), 0),
            change("auth.email_code.enabled", json!(true), 0),
        ]))
        .await
        .unwrap();
        let before = history(db).await.len();

        let err = svc
            .update_guarded(
                request(vec![change("smtp.port", json!(2525), 0)]),
                vec![reset("smtp.host", 1)],
                recording_guard(db),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(&err, SettingsError::Invalid { problems }
                if problems.iter().any(|p| p.key == "auth.email_code.enabled")),
            "{}: {err:?}",
            db.backend
        );
        assert_eq!(history(db).await.len(), before, "{}", db.backend);
        assert_eq!(svc.snapshot().get(&settings::SMTP_HOST), "mail.example.com");
        assert_eq!(svc.snapshot().get(&settings::SMTP_PORT), 587);
        assert!(audit_rows(db).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn versions_continue_after_a_reset_so_old_ones_never_match_again() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "auth.session_ttl", json!("60d"), 0).await;
        svc.update_guarded(
            request(Vec::new()),
            vec![reset("auth.session_ttl", 1)],
            recording_guard(db),
        )
        .await
        .unwrap();

        // Written again from "no row" (version 0): continues after the reset's entry.
        let next = svc
            .update(request(vec![change("auth.session_ttl", json!("45d"), 0)]))
            .await
            .unwrap();
        let view = next.view("auth.session_ttl").unwrap();
        assert_eq!(view.version, 3, "{}", db.backend);
        let stored = setting::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(stored[0].version, 3, "{}", db.backend);

        // A client that read version 1 before the reset must not overwrite the new row.
        let err = svc
            .update(request(vec![change("auth.session_ttl", json!("10d"), 1)]))
            .await
            .unwrap_err();
        assert!(
            matches!(err, SettingsError::VersionConflict { .. }),
            "{}: {err:?}",
            db.backend
        );
        let versions: Vec<i64> = history(db).await.iter().map(|h| h.version).collect();
        assert_eq!(versions, [1, 2, 3], "{}", db.backend);
        // The in-memory snapshot and a reload agree.
        let reloaded = service(db).await.snapshot();
        assert_eq!(
            reloaded.view("auth.session_ttl").unwrap(),
            next.view("auth.session_ttl").unwrap(),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn resetting_a_secret_records_only_the_set_marker() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "smtp.password", json!(PLAINTEXT), 0).await;

        svc.update_guarded(
            request(Vec::new()),
            vec![reset("smtp.password", 1)],
            recording_guard(db),
        )
        .await
        .unwrap();

        assert_eq!(stored_rows(db).await, 0, "{}", db.backend);
        let rows = history(db).await;
        assert_eq!(
            (
                rows[1].old_value.as_deref(),
                rows[1].new_value.as_str(),
                rows[1].reset
            ),
            (Some(r#"{"set":true}"#), r#"{"set":false}"#, true),
            "{}",
            db.backend
        );
        let everything = format!("{rows:?}{:?}", audit_rows(db).await);
        assert!(!everything.contains(PLAINTEXT), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn the_guard_sees_before_and_after_values_and_its_row_commits_with_the_batch() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "auth.session_ttl", json!("60d"), 0).await;

        svc.update_guarded(
            request(vec![
                change("smtp.password", json!(PLAINTEXT), 0),
                change("smtp.port", json!(2525), 0),
            ]),
            vec![reset("auth.session_ttl", 1)],
            recording_guard(db),
        )
        .await
        .unwrap();

        let rows = audit_rows(db).await;
        assert_eq!(rows.len(), 1, "{}", db.backend);
        let after: Json = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        let find = |key: &str| {
            after
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["key"] == key)
                .unwrap()
        };
        assert_eq!(
            find("auth.session_ttl"),
            &json!({ "key": "auth.session_ttl", "before": "60d", "after": "30d", "reset": true }),
            "{}",
            db.backend
        );
        assert_eq!(
            find("smtp.port"),
            &json!({ "key": "smtp.port", "before": 587, "after": 2525, "reset": false })
        );
        assert_eq!(
            find("smtp.password"),
            &json!({
                "key": "smtp.password",
                "before": { "set": false },
                "after": { "set": true },
                "reset": false,
            })
        );
        assert!(
            !rows[0].after.as_deref().unwrap().contains(PLAINTEXT),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_refusing_guard_rolls_the_whole_batch_back() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let guard = TxGuard::new(&audit_log(db), |_: Option<&AppliedChanges>, _| {
            Err(Refused::new("forbidden"))
        });

        let err = svc
            .update_guarded(
                request(vec![change("smtp.port", json!(2525), 0)]),
                Vec::new(),
                guard,
            )
            .await
            .unwrap_err();

        assert!(
            matches!(&err, SettingsError::Refused(r) if r.code == "forbidden"),
            "{}: {err:?}",
            db.backend
        );
        assert_eq!(stored_rows(db).await, 0, "{}", db.backend);
        assert!(history(db).await.is_empty(), "{}", db.backend);
        assert_eq!(svc.snapshot().get(&settings::SMTP_PORT), 587);
    })
    .await;
}

/// A subscriber of `svc`, with nothing pending yet.
fn subscriber(svc: &SettingsService) -> tokio::sync::watch::Receiver<Arc<settings::Snapshot>> {
    let mut rx = svc.subscribe();
    rx.mark_unchanged();
    rx
}

#[tokio::test]
async fn failed_batches_publish_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        set(&svc, "auth.session_ttl", json!("60d"), 0).await;
        let refusing = TxGuard::new(&audit_log(db), |_: Option<&AppliedChanges>, _| {
            Err(Refused::new("forbidden"))
        });
        let mut rx = subscriber(&svc);
        let before = rx.borrow().clone();

        // Refused by the guard, after the rows were written.
        let refused = svc
            .update_guarded(
                request(vec![change("smtp.port", json!(2525), 0)]),
                Vec::new(),
                refusing,
            )
            .await;
        // Stale version.
        let stale = svc
            .update_guarded(
                request(vec![change("auth.session_ttl", json!("90d"), 0)]),
                Vec::new(),
                recording_guard(db),
            )
            .await;
        // A cross-key rule that does not hold on the candidate.
        let rule = svc
            .update_guarded(
                request(vec![change("auth.email_code.enabled", json!(true), 0)]),
                Vec::new(),
                recording_guard(db),
            )
            .await;

        assert!(
            matches!(refused, Err(SettingsError::Refused(_))),
            "{}",
            db.backend
        );
        assert!(matches!(stale, Err(SettingsError::VersionConflict { .. })));
        assert!(matches!(rule, Err(SettingsError::Invalid { .. })));
        assert!(!rx.has_changed().unwrap(), "{}", db.backend);
        assert_eq!(*rx.borrow(), before, "{}", db.backend);
        assert_eq!(*svc.snapshot(), *before, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn committed_updates_and_resets_deliver_their_snapshot_to_subscribers() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        let mut rx = subscriber(&svc);

        let updated = svc
            .update_guarded(
                request(vec![change("auth.session_ttl", json!("60d"), 0)]),
                Vec::new(),
                recording_guard(db),
            )
            .await
            .unwrap();

        assert!(rx.has_changed().unwrap(), "{}", db.backend);
        assert_eq!(*rx.borrow_and_update(), updated, "{}", db.backend);
        assert_eq!(
            rx.borrow().view("auth.session_ttl").unwrap().value,
            json!("60d")
        );

        let reset_to_default = svc
            .update_guarded(
                request(Vec::new()),
                vec![reset("auth.session_ttl", 1)],
                recording_guard(db),
            )
            .await
            .unwrap();

        assert!(rx.has_changed().unwrap(), "{}", db.backend);
        assert_eq!(*rx.borrow_and_update(), reset_to_default, "{}", db.backend);
        let view = rx.borrow().view("auth.session_ttl").unwrap();
        assert_eq!((view.value, view.source), (json!("30d"), Source::Default));
    })
    .await;
}

#[tokio::test]
async fn history_pages_newest_first_and_masks_secrets() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db).await;
        for (value, version) in [("10d", 0), ("20d", 1), ("30d", 2)] {
            set(&svc, "auth.session_ttl", json!(value), version).await;
        }
        svc.update_guarded(
            request(Vec::new()),
            vec![reset("auth.session_ttl", 3)],
            recording_guard(db),
        )
        .await
        .unwrap();
        set(&svc, "smtp.password", json!(PLAINTEXT), 0).await;

        let first = svc
            .history(
                "auth.session_ttl",
                PageRequest {
                    page: 1,
                    page_size: 3,
                },
            )
            .await
            .unwrap();
        let second = svc
            .history(
                "auth.session_ttl",
                PageRequest {
                    page: 2,
                    page_size: 3,
                },
            )
            .await
            .unwrap();

        assert_eq!((first.total, first.items.len()), (4, 3), "{}", db.backend);
        let shown: Vec<_> = first
            .items
            .iter()
            .map(|e| (e.version, e.new_value.clone(), e.reset))
            .collect();
        assert_eq!(
            shown,
            [
                (4, json!("30d"), true),
                (3, json!("30d"), false),
                (2, json!("20d"), false)
            ],
            "{}",
            db.backend
        );
        assert_eq!(first.items[0].old_value, Some(json!("30d")));
        assert_eq!(first.items[0].updated_by, Some(ACTOR));
        assert_eq!(first.items[0].updated_by_username, None);
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].old_value, None);

        let secret = svc
            .history("smtp.password", PageRequest::default())
            .await
            .unwrap();
        assert_eq!(secret.items[0].new_value, json!({ "set": true }));
        assert!(!format!("{secret:?}").contains(PLAINTEXT));
        assert!(
            svc.history("nope", PageRequest::default()).await.is_err(),
            "{}",
            db.backend
        );
        assert!(
            svc.history(
                "smtp.port",
                PageRequest {
                    page: 0,
                    page_size: 1
                }
            )
            .await
            .is_err()
        );
    })
    .await;
}
