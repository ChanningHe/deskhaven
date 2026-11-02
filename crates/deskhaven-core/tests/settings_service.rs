//! Settings persistence: load, batch update, optimistic locking, history,
//! encryption at rest, hot reload. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::Source;
use deskhaven_config::bootstrap::Pins;
use deskhaven_config::settings::{self, Change, PinSet};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::settings::{SettingsError, SettingsService, UpdateRequest, value_context};
use deskhaven_db::entity::{setting, setting_history};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, PaginatorTrait, QueryOrder};
use serde_json::{Value as Json, json};

const PLAINTEXT: &str = "smtp-pass-0042";
const ACTOR: i64 = 42;

fn crypto() -> Arc<SecretBox> {
    Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])))
}

fn pins(raw: Json) -> PinSet {
    let pins: Pins = serde_json::from_value(raw).unwrap();
    PinSet::from_bootstrap(&pins).unwrap()
}

async fn service(db: &TestDb, pins: PinSet) -> SettingsService {
    let (service, warnings) = SettingsService::load(db.conn.clone(), crypto(), pins)
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

fn request(changes: Vec<Change>) -> UpdateRequest {
    UpdateRequest {
        changes,
        actor: Some(ACTOR),
        reason: Some("tuning".to_owned()),
    }
}

async fn row_count(db: &TestDb) -> (u64, u64) {
    let settings = setting::Entity::find().count(&db.conn).await.unwrap();
    let history = setting_history::Entity::find()
        .count(&db.conn)
        .await
        .unwrap();
    (settings, history)
}

async fn history(db: &TestDb) -> Vec<setting_history::Model> {
    setting_history::Entity::find()
        .order_by_asc(setting_history::Column::Id)
        .all(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn empty_database_yields_defaults() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;

        let snapshot = svc.snapshot();
        assert_eq!(snapshot.get(&settings::SMTP_PORT), 587);
        let view = snapshot.view("smtp.port").unwrap();
        assert_eq!(view.source, Source::Default);
        assert_eq!(view.version, 0);
    })
    .await;
}

#[tokio::test]
async fn update_persists_publishes_and_survives_reload() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;

        let next = svc
            .update(request(vec![change("auth.session_ttl", json!("60d"), 0)]))
            .await
            .unwrap();

        let ttl = |s: &settings::Snapshot| s.get(&settings::AUTH_SESSION_TTL).as_std().as_secs();
        assert_eq!(ttl(&next), 60 * 86_400);
        assert_eq!(ttl(&svc.snapshot()), 60 * 86_400);
        let Source::Database { version, by, at } = next.view("auth.session_ttl").unwrap().source
        else {
            panic!("expected a database source");
        };
        assert_eq!((version, by), (1, Some(ACTOR)));
        assert!(at > 0);

        let reloaded = service(db, PinSet::empty()).await.snapshot();
        assert_eq!(ttl(&reloaded), 60 * 86_400);
        assert_eq!(
            reloaded.view("auth.session_ttl").unwrap().source,
            next.view("auth.session_ttl").unwrap().source
        );
    })
    .await;
}

#[tokio::test]
async fn pin_overrides_stored_value_and_rejects_updates() {
    each_migrated_backend(async |db: &TestDb| {
        let free = service(db, PinSet::empty()).await;
        free.update(request(vec![change("auth.registration", json!("open"), 0)]))
            .await
            .unwrap();

        let pinned = service(db, pins(json!({ "auth": { "registration": "invite" } }))).await;

        let snapshot = pinned.snapshot();
        assert_eq!(
            snapshot.get(&settings::AUTH_REGISTRATION),
            settings::Registration::Invite
        );
        assert!(snapshot.view("auth.registration").unwrap().pinned);
        let err = pinned
            .update(request(vec![
                change("auth.lockout.window", json!("30m"), 0),
                change("auth.registration", json!("disabled"), 1),
            ]))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, SettingsError::Pinned { keys } if keys == &["auth.registration"]),
            "{err:?}"
        );
        assert_eq!(row_count(db).await, (1, 1), "nothing new written");
    })
    .await;
}

#[tokio::test]
async fn stale_version_is_a_conflict() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        let set = |v: i64| request(vec![change("smtp.port", json!(2525), v)]);

        svc.update(set(0)).await.unwrap();
        let err = svc.update(set(0)).await.unwrap_err();
        assert!(
            matches!(&err, SettingsError::VersionConflict { keys } if keys == &["smtp.port"]),
            "{err:?}"
        );
        let next = svc.update(set(1)).await.unwrap();
        assert_eq!(next.view("smtp.port").unwrap().version, 2);
    })
    .await;
}

const CONTENDERS: usize = 8;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_updates_of_one_key_yield_one_winner() {
    each_migrated_backend(async |db: &TestDb| {
        // One service per contender: the database, not the in-process writer
        // lock, must serialize them. All start from version 0.
        let mut services = Vec::with_capacity(CONTENDERS);
        for _ in 0..CONTENDERS {
            services.push(Arc::new(service(db, PinSet::empty()).await));
        }
        let barrier = Arc::new(tokio::sync::Barrier::new(CONTENDERS));
        let tasks: Vec<_> = services
            .iter()
            .zip(1u32..)
            .map(|(svc, port)| {
                let (svc, barrier) = (Arc::clone(svc), Arc::clone(&barrier));
                tokio::spawn(async move {
                    barrier.wait().await;
                    svc.update(request(vec![change("smtp.port", json!(port), 0)]))
                        .await
                })
            })
            .collect();

        let mut results = Vec::with_capacity(CONTENDERS);
        for task in tasks {
            results.push(task.await.unwrap());
        }

        let ok = results.iter().filter(|r| r.is_ok()).count();
        let conflicts = results
            .iter()
            .filter(|r| matches!(r, Err(SettingsError::VersionConflict { .. })))
            .count();
        assert_eq!(
            (ok, conflicts),
            (1, CONTENDERS - 1),
            "{}: {results:?}",
            db.backend
        );
        assert_eq!(row_count(db).await, (1, 1));
    })
    .await;
}

#[tokio::test]
async fn invalid_batch_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;

        let invalid_value = svc
            .update(request(vec![
                change("smtp.port", json!(2525), 0),
                change("auth.password.min_length", json!(4), 0),
            ]))
            .await;
        let cross_key = svc
            .update(request(vec![
                change("smtp.host", json!("mail.example.com"), 0),
                change("auth.email_code.enabled", json!(true), 0),
            ]))
            .await;
        let stale_second = svc
            .update(request(vec![
                change("smtp.port", json!(2525), 0),
                change("smtp.tls", json!("tls"), 3),
            ]))
            .await;

        assert!(
            matches!(invalid_value, Err(SettingsError::Invalid { .. })),
            "{invalid_value:?}"
        );
        assert!(
            matches!(cross_key, Err(SettingsError::Invalid { .. })),
            "{cross_key:?}"
        );
        assert!(
            matches!(stale_second, Err(SettingsError::VersionConflict { .. })),
            "{stale_second:?}"
        );
        assert_eq!(row_count(db).await, (0, 0));
        assert_eq!(svc.snapshot().get(&settings::SMTP_PORT), 587);
    })
    .await;
}

#[tokio::test]
async fn cross_key_rule_passes_when_satisfied_in_one_batch() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;

        let next = svc
            .update(request(vec![
                change("smtp.host", json!("mail.example.com"), 0),
                change("smtp.from", json!("rd@example.com"), 0),
                change("auth.email_code.enabled", json!(true), 0),
            ]))
            .await
            .unwrap();

        assert!(next.get(&settings::AUTH_EMAIL_CODE_ENABLED));
        assert_eq!(row_count(db).await, (3, 3));
    })
    .await;
}

#[tokio::test]
async fn history_records_actor_reason_and_values() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        svc.update(request(vec![change("smtp.port", json!(2525), 0)]))
            .await
            .unwrap();
        svc.update(UpdateRequest {
            changes: vec![change("smtp.port", json!("465"), 1)],
            actor: None,
            reason: None,
        })
        .await
        .unwrap();

        let rows = history(db).await;

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "smtp.port");
        assert_eq!(rows[0].old_value, None);
        assert_eq!(rows[0].new_value, "2525");
        assert_eq!(rows[0].version, 1);
        assert_eq!(rows[0].updated_by, Some(ACTOR));
        assert_eq!(rows[0].reason.as_deref(), Some("tuning"));
        assert_eq!(rows[1].old_value.as_deref(), Some("2525"));
        assert_eq!(rows[1].new_value, "465");
        assert_eq!((rows[1].version, rows[1].updated_by), (2, None));
        assert_eq!(rows[1].reason, None);
    })
    .await;
}

#[tokio::test]
async fn sensitive_value_is_encrypted_at_rest_and_never_shown() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;

        let next = svc
            .update(request(vec![change("smtp.password", json!(PLAINTEXT), 0)]))
            .await
            .unwrap();

        let row = setting::Entity::find_by_id("smtp.password")
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(!row.value.contains(PLAINTEXT), "plaintext at rest");
        let sealed: String = serde_json::from_str(&row.value).unwrap();
        let crypto = crypto();
        let opened = crypto
            .open(&value_context("smtp.password"), &sealed)
            .unwrap();
        assert_eq!(opened.as_slice(), PLAINTEXT.as_bytes());
        assert!(crypto.open(&value_context("smtp.host"), &sealed).is_err());

        let view = next.view("smtp.password").unwrap();
        assert_eq!(view.value, json!({ "set": true }));
        let all = serde_json::to_string(&svc.snapshot().views()).unwrap();
        assert!(!all.contains(PLAINTEXT) && !all.contains(&sealed));

        let rows = history(db).await;
        assert_eq!(rows[0].new_value, r#"{"set":true}"#);
        assert!(!rows[0].new_value.contains(&sealed));

        let reloaded = service(db, PinSet::empty()).await.snapshot();
        assert_eq!(reloaded.get(&settings::SMTP_PASSWORD).expose(), PLAINTEXT);
    })
    .await;
}

#[tokio::test]
async fn moved_or_corrupt_rows_fall_back_with_warnings() {
    each_migrated_backend(async |db: &TestDb| {
        // A secret sealed for another key (as if copied from that row), and
        // a row that is not JSON.
        let sealed = crypto()
            .seal(&value_context("smtp.username"), PLAINTEXT.as_bytes())
            .unwrap();
        let raw = |key: &str, value: String| setting::ActiveModel {
            key: Set(key.to_owned()),
            value: Set(value),
            version: Set(1),
            updated_by: Set(None),
            updated_at: Set(deskhaven_db::time::now()),
        };
        raw("smtp.password", Json::String(sealed.clone()).to_string())
            .insert(&db.conn)
            .await
            .unwrap();
        raw("smtp.port", "not json".to_owned())
            .insert(&db.conn)
            .await
            .unwrap();

        let (svc, warnings) = SettingsService::load(db.conn.clone(), crypto(), PinSet::empty())
            .await
            .unwrap();

        let mut keys: Vec<&str> = warnings.iter().map(|w| w.key.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["smtp.password", "smtp.port"], "{warnings:?}");
        let text = format!("{warnings:?}");
        assert!(
            !text.contains(&sealed) && !text.contains(PLAINTEXT),
            "{text}"
        );
        let snapshot = svc.snapshot();
        assert!(snapshot.get(&settings::SMTP_PASSWORD).is_empty());
        assert_eq!(snapshot.get(&settings::SMTP_PORT), 587);

        // The unreadable rows can be overwritten with the version the read
        // API reports for them.
        let version = |key: &str| snapshot.view(key).unwrap().version;
        assert_eq!((version("smtp.password"), version("smtp.port")), (1, 1));
        svc.update(request(vec![
            change(
                "smtp.password",
                json!("fresh-secret"),
                version("smtp.password"),
            ),
            change("smtp.port", json!(2525), version("smtp.port")),
        ]))
        .await
        .unwrap();
        let rows = history(db).await;
        assert_eq!(rows[0].old_value.as_deref(), Some(r#"{"set":false}"#));
        assert_eq!(rows[1].old_value.as_deref(), Some("not json"));
    })
    .await;
}

#[tokio::test]
async fn subscribers_see_the_new_snapshot() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        let mut rx = svc.subscribe();

        svc.update(request(vec![change(
            "signal.relay_servers",
            json!("r1:21117,r2:21117"),
            0,
        )]))
        .await
        .unwrap();

        rx.changed().await.unwrap();
        let relays = rx.borrow_and_update().get(&settings::SIGNAL_RELAY_SERVERS);
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[0].to_string(), "r1:21117");
    })
    .await;
}

#[tokio::test]
async fn dropped_update_still_commits_and_publishes_together() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        let mut rx = svc.subscribe();

        // Polled once (the batch is handed to its own task), then dropped.
        let update = svc.update(request(vec![change("smtp.port", json!(2525), 0)]));
        let dropped = tokio::time::timeout(std::time::Duration::ZERO, update).await;
        assert!(dropped.is_err(), "expected the caller to give up first");

        rx.changed().await.unwrap();
        assert_eq!(svc.snapshot().get(&settings::SMTP_PORT), 2525);
        assert_eq!(row_count(db).await, (1, 1));
    })
    .await;
}

#[tokio::test]
async fn overlong_or_multiline_reason_is_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        for reason in ["x".repeat(501), "line\nbreak".to_owned()] {
            let err = svc
                .update(UpdateRequest {
                    changes: vec![change("smtp.port", json!(2525), 0)],
                    actor: None,
                    reason: Some(reason),
                })
                .await
                .unwrap_err();
            assert!(matches!(err, SettingsError::Invalid { .. }), "{err:?}");
        }
        assert_eq!(row_count(db).await, (0, 0));
    })
    .await;
}

#[tokio::test]
async fn empty_batch_is_a_no_op() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service(db, PinSet::empty()).await;
        let before = svc.snapshot();

        let after = svc.update(request(Vec::new())).await.unwrap();

        assert!(Arc::ptr_eq(&before, &after));
        assert_eq!(row_count(db).await, (0, 0));
    })
    .await;
}
