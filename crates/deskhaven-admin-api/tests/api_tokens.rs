//! API token endpoints (M3-12): create (the secret appears once), list, revoke, the audit
//! rows without the secret, validation, and the acceptance: a revoked token is a 401 at
//! the real `--deploy` endpoint. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{Method, StatusCode};
use common::{Console, Harness, Reply, harness, request};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::api_tokens::{NewApiToken, Scope};
use deskhaven_core::users::Role;
use deskhaven_db::entity::{api_token, console_audit, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use tower::ServiceExt;

const TOKENS: &str = "/admin/api/v1/api-tokens";
const HOUR_MS: i64 = 3_600_000;

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

fn now_ms() -> i64 {
    time::now().timestamp_millis()
}

async fn create(h: &Harness, console: &Console, name: &str, scopes: &[&str]) -> Value {
    let body = json!({ "name": name, "scopes": scopes, "expires_at": now_ms() + HOUR_MS });
    let reply = h.write(Method::POST, TOKENS, console, body).await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body
}

fn id_of(created: &Value) -> i64 {
    created["token"]["id"].as_i64().unwrap()
}

fn revoke_path(id: i64) -> String {
    format!("{TOKENS}/{id}/revoke")
}

/// `POST /api/devices/deploy` through the real client API stack with `token`.
async fn deploy(h: &Harness, token: &str) -> (StatusCode, Value) {
    let router = apply_middleware(
        deskhaven_client_api::router().with_state(Arc::clone(&h.state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_client_api::render_stack_error),
    );
    let bearer = format!("Bearer {token}");
    let body = json!({ "id": "123456789", "uuid": "dXVpZA==", "pk": "AAAA" });
    let headers = [("authorization", bearer.as_str())];
    let req = request(Method::POST, "/api/devices/deploy", &headers, Some(body));
    let response = router.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A token without creator, as a future CLI would issue one.
async fn system_token(h: &Harness) -> i64 {
    let new = NewApiToken {
        name: "system".to_owned(),
        scopes: [Scope::Deploy].into(),
        created_by: None,
        expires_at: None,
    };
    h.state.api_tokens.create(new).await.unwrap().0.id
}

fn names(reply: &Reply) -> Vec<String> {
    let items = reply.body["items"].as_array().unwrap();
    let name = |t: &Value| t["name"].as_str().unwrap().to_owned();
    items.iter().map(name).collect()
}

fn snapshot(text: &Option<String>) -> Value {
    serde_json::from_str(text.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn a_created_token_shows_its_secret_once_and_is_listed_without_it() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;

        let created = create(&h, &admin, "ci deploy", &["deploy", "assign"]).await;

        let secret = created["secret"].as_str().unwrap();
        assert!(secret.starts_with("dhk_"), "{}", db.backend);
        let token = &created["token"];
        assert_eq!(token["name"], "ci deploy");
        assert_eq!(token["scopes"], json!(["deploy", "assign"]));
        assert_eq!(token["status"], "active");
        assert_eq!(token["created_by"]["username"], "adm");
        assert_eq!(token["created_by"]["role"], "admin");
        assert!(secret.starts_with(token["prefix"].as_str().unwrap()));
        let listed = h.get(TOKENS, &admin).await;
        assert_eq!(listed.status, 200);
        assert_eq!(listed.body["total"], 1);
        let dump = listed.body.to_string();
        assert!(!dump.contains(secret), "{}", db.backend);
        assert!(!dump.contains("hash"), "{}", db.backend);
        let item = &listed.body["items"][0];
        assert!(item["last_used_at"].is_null() && item["revoked_at"].is_null());
    })
    .await;
}

#[tokio::test]
async fn the_secret_response_is_not_cacheable() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let body = json!({ "name": "t", "scopes": ["deploy"] });

        let reply = h.write(Method::POST, TOKENS, &admin, body).await;

        assert_eq!(reply.status, 201);
        assert_eq!(reply.headers["cache-control"], "no-store");
        assert!(reply.body["token"]["expires_at"].is_null());
    })
    .await;
}

#[tokio::test]
async fn create_and_revoke_are_audited_without_the_secret_or_the_hash() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let created = create(&h, &admin, "ci deploy", &["deploy"]).await;
        let id = id_of(&created);
        let secret = created["secret"].as_str().unwrap().to_owned();
        let revoked = h.act(Method::POST, &revoke_path(id), &admin).await;
        assert_eq!(revoked.status, 200, "{}", revoked.body);

        let rows = h.audit_rows().await;

        let shape: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.action.as_str(),
                    r.target_type.as_str(),
                    r.target_id.as_str(),
                )
            })
            .collect();
        let target = id.to_string();
        assert_eq!(
            shape,
            [
                ("api_token.create", "api_token", target.as_str()),
                ("api_token.revoke", "api_token", target.as_str()),
            ]
        );
        let (made, revoke) = (&rows[0], &rows[1]);
        assert_eq!(made.actor_kind, console_audit::ActorKind::User);
        assert_eq!(made.actor_id, Some(admin.user_id));
        assert!(made.before.is_none() && revoke.before.is_some());
        let after = snapshot(&made.after);
        assert_eq!(after["name"], "ci deploy");
        assert_eq!(after["scopes"], json!(["deploy"]));
        assert_eq!(after["created_by"], admin.user_id);
        assert!(after["expires_at"].is_i64());
        assert_eq!(after["revoked"], false);
        assert_eq!(snapshot(&revoke.after)["revoked"], true);
        let stored = api_token::Entity::find_by_id(id)
            .one(&h.state.db)
            .await
            .unwrap()
            .unwrap();
        for row in &rows {
            let dump = format!("{:?} {:?}", row.before, row.after);
            assert!(!dump.contains(&secret), "{}", db.backend);
            assert!(!dump.contains(&stored.token_hash), "{}", db.backend);
            assert!(!dump.contains("dhk_"), "{}", db.backend);
        }
        assert!(
            after["prefix"].as_str().unwrap().len() >= 4,
            "identifies the token"
        );
    })
    .await;
}

#[tokio::test]
async fn a_revoked_token_is_a_401_at_the_deploy_endpoint() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let created = create(&h, &admin, "deploy", &["deploy"]).await;
        let secret = created["secret"].as_str().unwrap().to_owned();
        let (before, _) = deploy(&h, &secret).await;
        assert_eq!(before, StatusCode::OK, "{}", db.backend);

        let revoked = h
            .act(Method::POST, &revoke_path(id_of(&created)), &admin)
            .await;

        assert_eq!(revoked.status, 200, "{}", revoked.body);
        assert_eq!(revoked.body["status"], "revoked");
        let (after, body) = deploy(&h, &secret).await;
        assert_eq!(after, StatusCode::UNAUTHORIZED, "{}", db.backend);
        assert_eq!(body["error"], "Unauthorized");
    })
    .await;
}

#[tokio::test]
async fn revoking_twice_keeps_the_first_revocation_and_records_once() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = id_of(&create(&h, &admin, "t", &["deploy"]).await);
        let first = h.act(Method::POST, &revoke_path(id), &admin).await;

        let second = h.act(Method::POST, &revoke_path(id), &admin).await;

        assert_eq!(second.status, 200, "{}", second.body);
        assert_eq!(second.body["revoked_at"], first.body["revoked_at"]);
        assert_eq!(h.audit_rows().await.len(), 2, "create + one revoke");
        let missing = h.act(Method::POST, &revoke_path(id + 99), &admin).await;
        assert_eq!(
            (missing.status.as_u16(), missing.code()),
            (404, "not_found")
        );
    })
    .await;
}

#[tokio::test]
async fn create_rejects_bad_input_and_records_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let past = now_ms() - HOUR_MS;
        let cases = [
            json!({ "name": " ", "scopes": ["deploy"] }),
            json!({ "name": "x".repeat(65), "scopes": ["deploy"] }),
            json!({ "name": "t", "scopes": [] }),
            json!({ "name": "t", "scopes": ["root"] }),
            json!({ "name": "t", "scopes": ["deploy"], "expires_at": past }),
            json!({ "name": "t", "scopes": ["deploy"], "extra": 1 }),
            json!({ "scopes": ["deploy"] }),
        ];

        for body in cases {
            let reply = h.write(Method::POST, TOKENS, &admin, body.clone()).await;
            assert_eq!(reply.status, 422, "{body}: {}", reply.body);
            assert_eq!(reply.code(), "validation_failed");
        }
        assert!(h.audit_rows().await.is_empty());
        assert_eq!(h.get(TOKENS, &admin).await.body["total"], 0);
    })
    .await;
}

#[tokio::test]
async fn a_demoted_creator_with_a_stale_session_gets_403_not_401() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        user::Entity::update_many()
            .col_expr(user::Column::Role, Expr::value("user"))
            .filter(user::Column::Id.eq(admin.user_id))
            .exec(&h.state.db)
            .await
            .unwrap();

        let body = json!({ "name": "t", "scopes": ["deploy"] });
        let reply = h.write(Method::POST, TOKENS, &admin, body).await;

        let got = (reply.status.as_u16(), reply.code());
        assert_eq!(got, (403, "forbidden"), "{}", db.backend);
        assert!(h.audit_rows().await.is_empty());
        let tokens = api_token::Entity::find().all(&h.state.db).await.unwrap();
        assert!(tokens.is_empty(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn only_administrators_reach_the_endpoints() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let user = h.console("plain", Role::User).await;
        let id = id_of(&create(&h, &admin, "t", &["deploy"]).await);
        let body = json!({ "name": "t", "scopes": ["deploy"] });

        let list = h.get(TOKENS, &user).await;
        let post = h.write(Method::POST, TOKENS, &user, body.clone()).await;
        let revoke = h.act(Method::POST, &revoke_path(id), &user).await;
        let anonymous = h.send(Method::GET, TOKENS, None, None, None).await;
        let cookie = Some(admin.cookie.as_str());
        let no_csrf = h.send(Method::POST, TOKENS, cookie, None, Some(body)).await;

        for reply in [&list, &post, &revoke] {
            let got = (reply.status.as_u16(), reply.code());
            assert_eq!(got, (403, "forbidden"), "{}", db.backend);
        }
        assert_eq!(anonymous.status, 401);
        assert_eq!(
            (no_csrf.status.as_u16(), no_csrf.code()),
            (403, "csrf_failed")
        );
        let still = h.get(TOKENS, &admin).await;
        assert_eq!(still.body["items"][0]["status"], "active");
    })
    .await;
}

/// 06 §4.1: tokens carry their creator's authority, so an owner's tokens (and system
/// tokens, which have no creator) are an owner's to revoke; admins revoke each other's.
#[tokio::test]
async fn admins_cannot_revoke_owner_or_system_tokens() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let owner = h.console("boss", Role::Owner).await;
        let other = h.console("adm2", Role::Admin).await;
        let owners = create(&h, &owner, "owner token", &["deploy"]).await;
        let admins = create(&h, &other, "admin token", &["deploy"]).await;
        let system_id = system_token(&h).await;

        let on_owner = h
            .act(Method::POST, &revoke_path(id_of(&owners)), &admin)
            .await;
        let on_system = h.act(Method::POST, &revoke_path(system_id), &admin).await;
        let on_admin = h
            .act(Method::POST, &revoke_path(id_of(&admins)), &admin)
            .await;
        let by_owner = h.act(Method::POST, &revoke_path(system_id), &owner).await;

        for reply in [&on_owner, &on_system] {
            let denied = (reply.status.as_u16(), reply.code());
            assert_eq!(denied, (403, "owner_required"), "{}", db.backend);
        }
        assert_eq!(on_admin.status, 200, "{}", on_admin.body);
        assert_eq!(by_owner.status, 200, "{}", by_owner.body);
        let (status, _) = deploy(&h, owners["secret"].as_str().unwrap()).await;
        assert_eq!(status, StatusCode::OK, "the refused revoke left it usable");
        let listed = h.get(&format!("{TOKENS}?q=owner"), &admin).await;
        assert_eq!(listed.body["items"][0]["created_by"]["role"], "owner");
        let rows = h.audit_rows().await;
        let revokes = rows.iter().filter(|r| r.action == "api_token.revoke");
        assert_eq!(revokes.count(), 2, "refused revokes leave no row");
    })
    .await;
}

#[tokio::test]
async fn the_list_filters_sorts_and_pages() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        for name in ["beta", "Alpha", "gamma", "alpha-2"] {
            create(&h, &admin, name, &["assign"]).await;
        }

        let default = h.get(TOKENS, &admin).await;
        let by_name = h.get(&format!("{TOKENS}?sort=name"), &admin).await;
        let found = h.get(&format!("{TOKENS}?q=ALPHA&sort=-name"), &admin).await;
        let paged = h.get(&format!("{TOKENS}?page=2&page_size=3"), &admin).await;
        let bad = h.get(&format!("{TOKENS}?sort=scopes"), &admin).await;

        let newest_first = ["alpha-2", "gamma", "Alpha", "beta"];
        assert_eq!(names(&default), newest_first, "{}", db.backend);
        assert_eq!(names(&by_name), ["Alpha", "alpha-2", "beta", "gamma"]);
        assert_eq!(names(&found), ["alpha-2", "Alpha"]);
        assert_eq!(
            (paged.body["total"].as_u64(), names(&paged).len()),
            (Some(4), 1)
        );
        assert_eq!(bad.status, 422);
    })
    .await;
}

#[tokio::test]
async fn status_reflects_expiry() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = id_of(&create(&h, &admin, "t", &["deploy"]).await);
        let past = time::from_millis(now_ms() - HOUR_MS).unwrap();
        api_token::Entity::update_many()
            .col_expr(api_token::Column::ExpiresAt, Expr::value(past))
            .filter(api_token::Column::Id.eq(id))
            .exec(&h.state.db)
            .await
            .unwrap();

        let listed = h.get(TOKENS, &admin).await;

        assert_eq!(
            listed.body["items"][0]["status"], "expired",
            "{}",
            db.backend
        );
    })
    .await;
}
