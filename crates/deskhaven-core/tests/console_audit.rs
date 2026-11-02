//! `ConsoleAuditLog`: rows on both backends, actors, redaction and bounded labels.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::T0_MS;
use deskhaven_config::REDACTED;
use deskhaven_core::clock::ManualClock;
use deskhaven_core::console_audit::redact;
use deskhaven_core::console_audit::{
    Actor, ActorKind, ConsoleAuditLog, MAX_LABEL_CHARS, NewConsoleAudit,
};
use deskhaven_db::begin_write;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::Backend;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, Statement};
use serde_json::{Value, json};

fn entry(actor: Actor) -> NewConsoleAudit {
    NewConsoleAudit {
        actor,
        action: "user.update".to_owned(),
        target_type: "user".to_owned(),
        target_id: "42".to_owned(),
        before: Some(json!({ "display_name": "Old", "password": "hunter2" })),
        after: Some(json!({ "display_name": "New", "password": "hunter3" })),
        ip: Some("192.0.2.9".parse().unwrap()),
    }
}

async fn row(db: &TestDb, id: i64) -> console_audit::Model {
    console_audit::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

fn json_of(text: Option<String>) -> Value {
    serde_json::from_str(&text.unwrap()).unwrap()
}

#[tokio::test]
async fn records_actor_target_and_redacted_snapshots() {
    each_migrated_backend(async |db: &TestDb| {
        let log = ConsoleAuditLog::new(db.conn.clone(), Arc::new(ManualClock::at(T0_MS)));

        let id = log.record(&entry(Actor::User(7))).await.unwrap();

        let got = row(db, id).await;
        assert_eq!(got.actor_kind, ActorKind::User, "{}", db.backend);
        assert_eq!(got.actor_id, Some(7));
        assert_eq!(got.action, "user.update");
        assert_eq!(
            (got.target_type.as_str(), got.target_id.as_str()),
            ("user", "42")
        );
        assert_eq!(got.ip.as_deref(), Some("192.0.2.9"));
        assert_eq!(got.created_at.timestamp_millis(), T0_MS);
        assert_eq!(
            json_of(got.before),
            json!({ "display_name": "Old", "password": REDACTED })
        );
        assert_eq!(
            json_of(got.after),
            json!({ "display_name": "New", "password": REDACTED })
        );
    })
    .await;
}

#[tokio::test]
async fn every_actor_kind_satisfies_the_schema() {
    each_migrated_backend(async |db: &TestDb| {
        let log = ConsoleAuditLog::new(db.conn.clone(), Arc::new(ManualClock::at(T0_MS)));
        let cases = [
            (Actor::User(1), ActorKind::User, Some(1)),
            (Actor::ApiToken(2), ActorKind::ApiToken, Some(2)),
            (Actor::Cli, ActorKind::Cli, None),
            (Actor::System, ActorKind::System, None),
        ];
        for (actor, kind, actor_id) in cases {
            let id = log
                .record(&NewConsoleAudit {
                    before: None,
                    after: None,
                    ip: None,
                    ..entry(actor)
                })
                .await
                .unwrap();
            let got = row(db, id).await;
            assert_eq!(
                (got.actor_kind, got.actor_id),
                (kind, actor_id),
                "{actor:?}"
            );
            assert_eq!((got.before, got.after, got.ip), (None, None, None));
        }
    })
    .await;
}

#[tokio::test]
async fn over_long_labels_are_cut() {
    each_migrated_backend(async |db: &TestDb| {
        let log = ConsoleAuditLog::new(db.conn.clone(), Arc::new(ManualClock::at(T0_MS)));
        let long = "x".repeat(MAX_LABEL_CHARS * 2);

        let id = log
            .record(&NewConsoleAudit {
                target_id: long,
                ..entry(Actor::System)
            })
            .await
            .unwrap();

        assert_eq!(row(db, id).await.target_id.len(), MAX_LABEL_CHARS);
    })
    .await;
}

async fn count(db: &TestDb) -> u64 {
    console_audit::Entity::find().count(&db.conn).await.unwrap()
}

#[tokio::test]
async fn record_in_commits_and_rolls_back_with_the_caller_transaction() {
    each_migrated_backend(async |db: &TestDb| {
        let log = ConsoleAuditLog::new(db.conn.clone(), Arc::new(ManualClock::at(T0_MS)));

        let tx = begin_write(&db.conn).await.unwrap();
        log.record_in(&tx, &entry(Actor::User(1))).await.unwrap();
        tx.rollback().await.unwrap();
        assert_eq!(count(db).await, 0, "{}: rolled back row kept", db.backend);

        let tx = begin_write(&db.conn).await.unwrap();
        let id = log.record_in(&tx, &entry(Actor::User(1))).await.unwrap();
        tx.commit().await.unwrap();
        let got = row(db, id).await;
        assert_eq!(json_of(got.after)["password"], json!(REDACTED));
    })
    .await;
}

/// Every column of the migrated schema, as `(table, column)`.
async fn schema_columns(db: &TestDb) -> Vec<(String, String)> {
    let sql = match db.backend {
        Backend::Sqlite => {
            "SELECT m.name AS t, p.name AS c FROM sqlite_master m \
             JOIN pragma_table_info(m.name) p WHERE m.type = 'table'"
        }
        Backend::Postgres => {
            "SELECT table_name::text AS t, column_name::text AS c \
             FROM information_schema.columns WHERE table_schema = current_schema()"
        }
    };
    let backend = db.conn.get_database_backend();
    let rows = db
        .conn
        .query_all_raw(Statement::from_string(backend, sql))
        .await
        .unwrap();
    rows.iter()
        .map(|r| (r.try_get("", "t").unwrap(), r.try_get("", "c").unwrap()))
        .collect()
}

#[tokio::test]
async fn every_ciphertext_digest_or_secret_column_of_the_schema_is_redacted() {
    each_migrated_backend(async |db: &TestDb| {
        let sensitive: Vec<(String, String)> = schema_columns(db)
            .await
            .into_iter()
            .filter(|(_, c)| {
                c.ends_with("_enc")
                    || c.ends_with("_hash")
                    || ["secret", "password", "token"]
                        .iter()
                        .any(|f| c.contains(f))
            })
            .collect();

        for must in [
            "totp_secret_enc",
            "password_hash",
            "token_hash",
            "client_secret_enc",
        ] {
            assert!(
                sensitive.iter().any(|(_, c)| c == must),
                "{}: {must} missing from {sensitive:?}",
                db.backend
            );
        }
        for (table, column) in &sensitive {
            let got = redact(&json!({ column.as_str(): "v" }));
            assert_eq!(got[column.as_str()], json!(REDACTED), "{table}.{column}");
        }
    })
    .await;
}
