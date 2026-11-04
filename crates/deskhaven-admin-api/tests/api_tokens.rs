//! API token endpoints (M3-12): create (the secret appears once), list, revoke, the audit
//! rows without the secret, and the acceptance: a revoked token is a 401 at the real
//! `--deploy` endpoint. Runs on SQLite and PostgreSQL. Rules and validation:
//! `api_tokens_rules.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::tokens::{
    HOUR_MS, TOKENS, create, deploy, id_of, names, now_ms, revoke_path, setup, snapshot,
};
use deskhaven_db::entity::{api_token, console_audit};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

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
