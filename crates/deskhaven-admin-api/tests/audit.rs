//! Console audit of admin API writes (M3-01 acceptance: rows hold before/after values,
//! secrets masked).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, PEER, harness, probe};
use deskhaven_config::REDACTED;
use deskhaven_core::console_audit::ActorKind;
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ConnectionTrait, EntityTrait, QueryOrder};
use serde_json::{Value, json};

async fn app(db: &TestDb) -> Harness {
    harness(db, probe::routes()).await
}

async fn rows(db: &TestDb) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .order_by_asc(console_audit::Column::Id)
        .all(&db.conn)
        .await
        .unwrap()
}

fn json_of(text: &Option<String>) -> Value {
    serde_json::from_str(text.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn admin_write_records_actor_target_and_masked_before_after() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;

        let reply = app
            .write(Method::POST, "/admin/api/v1/probe/admin", &ada, json!({}))
            .await;

        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{}: {:?}",
            db.backend,
            reply.body
        );
        let [row] = rows(db).await.try_into().unwrap();
        assert_eq!(row.actor_kind, ActorKind::User);
        assert_eq!(row.actor_id, Some(ada.user_id));
        assert_eq!(row.action, "probe.update");
        assert_eq!(
            (row.target_type.as_str(), row.target_id.as_str()),
            ("probe", "7")
        );
        assert_eq!(row.ip, Some(PEER.ip().to_string()));
        assert_eq!(
            json_of(&row.before),
            json!({ "name": "old", "value": REDACTED, "password": REDACTED })
        );
        assert_eq!(
            json_of(&row.after),
            json!({ "name": "new", "value": REDACTED, "password": REDACTED })
        );
        let stored = format!("{:?}{:?}", row.before, row.after);
        for secret in ["old-secret", "new-secret", "hunter2", "hunter3"] {
            assert!(!stored.contains(secret), "{secret} leaked: {stored}");
        }
    })
    .await;
}

#[tokio::test]
async fn write_without_audit_details_records_the_operation_id() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let olga = app.console("olga", Role::Owner).await;

        let reply = app
            .write(
                Method::DELETE,
                "/admin/api/v1/probe/owner",
                &olga,
                json!({}),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK);
        let [row] = rows(db).await.try_into().unwrap();
        assert_eq!(row.actor_id, Some(olga.user_id));
        assert_eq!(row.action, "owner_write");
        assert_eq!((row.target_type.as_str(), row.target_id.as_str()), ("", ""));
        assert_eq!((row.before, row.after), (None, None));
    })
    .await;
}

#[tokio::test]
async fn reads_failed_denied_and_public_writes_are_not_audited() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let ada = app.console("ada", Role::Admin).await;

        let read = app.get("/admin/api/v1/me", &ada).await;
        let failed = app
            .write(Method::PUT, "/admin/api/v1/probe/failing", &ada, json!({}))
            .await;
        let denied = app
            .write(Method::POST, "/admin/api/v1/probe/admin", &uma, json!({}))
            .await;
        let no_csrf = app
            .send(
                Method::POST,
                "/admin/api/v1/probe/admin",
                Some(&ada.cookie),
                None,
                Some(json!({})),
            )
            .await;
        let public = app
            .send(
                Method::POST,
                "/admin/api/v1/probe/public",
                None,
                None,
                Some(json!({})),
            )
            .await;

        let statuses = [
            read.status,
            failed.status,
            denied.status,
            no_csrf.status,
            public.status,
        ];
        assert_eq!(
            statuses,
            [
                StatusCode::OK,
                StatusCode::CONFLICT,
                StatusCode::FORBIDDEN,
                StatusCode::FORBIDDEN,
                StatusCode::OK
            ]
        );
        assert!(rows(db).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn audit_write_failure_fails_the_request() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;
        db.conn
            .execute_unprepared("DROP TABLE console_audit")
            .await
            .unwrap();

        let reply = app
            .write(Method::POST, "/admin/api/v1/probe/admin", &ada, json!({}))
            .await;

        assert_eq!(
            reply.status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{}",
            db.backend
        );
        assert_eq!(
            reply.body,
            json!({ "error": { "code": "internal", "message": "internal server error" } })
        );
    })
    .await;
}
