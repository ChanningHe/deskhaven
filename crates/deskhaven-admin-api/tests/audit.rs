//! Console audit of admin API operations: in-transaction recording, the detached
//! fallback, `Audit` on reads and `Public` routes, masked before/after values.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{Harness, PEER, auth_headers, harness, probe, request};
use deskhaven_config::REDACTED;
use deskhaven_core::console_audit::ActorKind;
use deskhaven_core::users::Role;
use deskhaven_db::entity::{console_audit, group};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait, QueryOrder};
use serde_json::{Value, json};

/// How long a detached fallback write may take to land.
const LANDING: Duration = Duration::from_secs(5);

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

/// The audit rows once `n` exist (fallback rows are written by a detached task).
async fn rows_eventually(db: &TestDb, n: usize) -> Vec<console_audit::Model> {
    let deadline = tokio::time::Instant::now() + LANDING;
    loop {
        let found = rows(db).await;
        if found.len() >= n || tokio::time::Instant::now() > deadline {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Lets any spawned fallback task run, then the rows.
async fn rows_settled(db: &TestDb) -> Vec<console_audit::Model> {
    tokio::time::sleep(Duration::from_millis(200)).await;
    rows(db).await
}

fn json_of(text: &Option<String>) -> Value {
    serde_json::from_str(text.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn in_transaction_audit_commits_with_the_change() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;

        let reply = app
            .write(
                Method::POST,
                "/admin/api/v1/probe/atomic",
                &ada,
                json!({ "name": "ops", "fail": false }),
            )
            .await;

        assert_eq!(
            reply.status,
            StatusCode::OK,
            "{}: {:?}",
            db.backend,
            reply.body
        );
        let id = reply.body["id"].as_i64().unwrap();
        let [row] = rows_settled(db).await.try_into().unwrap();
        assert_eq!(
            (row.actor_kind, row.actor_id),
            (ActorKind::User, Some(ada.user_id))
        );
        assert_eq!(row.action, "group.create");
        assert_eq!(row.target_id, id.to_string());
        assert_eq!(row.ip, Some(PEER.ip().to_string()));
        assert_eq!(json_of(&row.after), json!({ "name": "ops" }));
    })
    .await;
}

#[tokio::test]
async fn rolled_back_change_leaves_no_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;

        let reply = app
            .write(
                Method::POST,
                "/admin/api/v1/probe/atomic",
                &ada,
                json!({ "name": "ops", "fail": true }),
            )
            .await;

        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", db.backend);
        assert!(rows_settled(db).await.is_empty());
        assert_eq!(group::Entity::find().count(&db.conn).await.unwrap(), 0);
    })
    .await;
}

#[tokio::test]
async fn fallback_row_lands_when_the_request_is_dropped_after_the_handler() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;
        let req = request(
            Method::POST,
            "/admin/api/v1/probe/fallback",
            &auth_headers(&ada, &[]),
            Some(json!({})),
        );

        // The deadline or a disconnect drops the request future: drop it as soon as the
        // handler has finished, before the response is delivered.
        let mut call = Box::pin(app.call(req));
        tokio::select! {
            biased;
            () = probe::FALLBACK_HANDLER_DONE.notified() => {}
            _ = &mut call => {}
        }
        drop(call);

        let found = rows_eventually(db, 1).await;
        let [row] = found
            .try_into()
            .unwrap_or_else(|f: Vec<_>| panic!("{}: {f:?}", db.backend));
        assert_eq!(row.action, "probe.fallback");
        assert_eq!(row.actor_id, Some(ada.user_id));
    })
    .await;
}

#[tokio::test]
async fn fallback_records_actor_target_and_masked_before_after() {
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
        let [row] = rows_eventually(db, 1).await.try_into().unwrap();
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
        let [row] = rows_eventually(db, 1).await.try_into().unwrap();
        assert_eq!(row.actor_id, Some(olga.user_id));
        assert_eq!(row.action, "owner_write");
        assert_eq!((row.target_type.as_str(), row.target_id.as_str()), ("", ""));
        assert_eq!((row.before, row.after), (None, None));
    })
    .await;
}

#[tokio::test]
async fn audit_attached_to_a_get_is_recorded() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;

        let reply = app.get("/admin/api/v1/probe/secret", &ada).await;

        assert_eq!(reply.status, StatusCode::OK);
        let [row] = rows_eventually(db, 1).await.try_into().unwrap();
        assert_eq!(row.action, "probe.secret.view", "{}", db.backend);
        assert_eq!(row.actor_id, Some(ada.user_id));
    })
    .await;
}

#[tokio::test]
async fn public_route_audit_uses_the_named_actor_or_the_console_user() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;

        let named = app
            .send(
                Method::POST,
                "/admin/api/v1/probe/public",
                None,
                Some("x"),
                Some(json!({})),
            )
            .await;
        let session = app
            .write(
                Method::POST,
                "/admin/api/v1/probe/public-session",
                &uma,
                json!({}),
            )
            .await;

        assert_eq!(
            (named.status, session.status),
            (StatusCode::OK, StatusCode::OK)
        );
        let found = rows_eventually(db, 2).await;
        let actions: Vec<(&str, ActorKind, Option<i64>)> = found
            .iter()
            .map(|r| (r.action.as_str(), r.actor_kind, r.actor_id))
            .collect();
        assert!(
            actions.contains(&("probe.public", ActorKind::User, Some(424_242))),
            "{actions:?}"
        );
        assert!(
            actions.contains(&("probe.public.session", ActorKind::User, Some(uma.user_id))),
            "{actions:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn reads_failed_and_denied_requests_are_not_audited() {
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

        let statuses = [read.status, failed.status, denied.status, no_csrf.status];
        assert_eq!(
            statuses,
            [
                StatusCode::OK,
                StatusCode::CONFLICT,
                StatusCode::FORBIDDEN,
                StatusCode::FORBIDDEN
            ]
        );
        assert!(rows_settled(db).await.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn fallback_write_failure_keeps_the_success_response() {
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

        assert_eq!(reply.status, StatusCode::OK, "{}", db.backend);
        assert_eq!(reply.body, json!({ "ok": true }));
        // Let the failing detached write finish before the database goes away.
        tokio::time::sleep(Duration::from_millis(200)).await;
    })
    .await;
}
