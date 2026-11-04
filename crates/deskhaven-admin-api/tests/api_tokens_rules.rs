//! API token rules (M3-12): input validation, who may create and revoke (06 §4.1), and
//! the lock order against account edits (PostgreSQL). Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::Harness;
use common::tokens::{
    HOUR_MS, TOKENS, create, deploy, id_of, now_ms, revoke_path, setup, system_token,
};
use deskhaven_core::users::Role;
use deskhaven_db::begin_write;
use deskhaven_db::entity::api_token;
use deskhaven_db::entity::user::{self, Status};
use deskhaven_db::testing::{Backend, TestDb, each_migrated_backend};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
    Statement,
};
use serde_json::json;

const TEN_YEARS_MS: i64 = 3650 * 24 * HOUR_MS;

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
async fn names_with_invisible_or_bidi_characters_are_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let names = [
            "a\u{200B}b",
            "gp\u{202E}dab",
            "\u{FEFF}",
            "ops\u{2066}",
            "tab\there",
        ];

        for name in names {
            let body = json!({ "name": name, "scopes": ["deploy"] });
            let reply = h.write(Method::POST, TOKENS, &admin, body).await;
            assert_eq!(reply.status, 422, "{name:?}: {}", reply.body);
        }
        let plain = json!({ "name": "CI 部署 – α", "scopes": ["deploy"] });
        let ok = h.write(Method::POST, TOKENS, &admin, plain).await;
        assert_eq!(ok.status, 201, "{}", ok.body);
    })
    .await;
}

#[tokio::test]
async fn an_expiry_beyond_ten_years_is_rejected_and_a_shorter_one_accepted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let at = |ms: i64| json!({ "name": "t", "scopes": ["deploy"], "expires_at": ms });

        let far = h
            .write(
                Method::POST,
                TOKENS,
                &admin,
                at(now_ms() + TEN_YEARS_MS + HOUR_MS),
            )
            .await;
        let near = h
            .write(
                Method::POST,
                TOKENS,
                &admin,
                at(now_ms() + TEN_YEARS_MS - HOUR_MS),
            )
            .await;

        assert_eq!(
            (far.status.as_u16(), far.code()),
            (422, "validation_failed")
        );
        assert!(far.body["error"]["details"]["fields"]["expires_at"].is_array());
        assert_eq!(near.status, 201, "{}", near.body);
    })
    .await;
}

/// How many backends of the test's database wait for a lock.
async fn lock_waiters(h: &Harness) -> i64 {
    let sql = "SELECT count(*) AS n FROM pg_stat_activity \
               WHERE datname = current_database() AND wait_event_type = 'Lock'";
    let stmt = Statement::from_string(DbBackend::Postgres, sql);
    let row = h.state.db.query_one_raw(stmt).await.unwrap().unwrap();
    row.try_get("", "n").unwrap()
}

async fn wait_for_lock_waiters(h: &Harness, count: i64) {
    for _ in 0..500 {
        if lock_waiters(h).await >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("fewer than {count} backends waited for a lock");
}

/// `UserService::edit` locks every active owner, then the edited account, then (guard) the
/// acting account. A revocation by an owner of the edited account's token must take the
/// same locks in the same order: with creator first, owner O (revoking) and the editor
/// would wait for each other and PostgreSQL would abort one of them.
#[tokio::test]
async fn revoking_a_token_while_its_creator_is_edited_does_not_deadlock() {
    each_migrated_backend(async |db: &TestDb| {
        if db.backend != Backend::Postgres {
            return;
        }
        let (h, creator) = setup(db).await;
        let revoker = h.console("boss", Role::Owner).await;
        let editor = h.console("boss2", Role::Owner).await;
        let token = id_of(&create(&h, &creator, "t", &["deploy"]).await);
        // Stands for a third transaction holding the owners, so both requests queue
        // behind it and then run in the order they queued: edit first.
        let blocker = begin_write(&h.state.db).await.unwrap();
        user::Entity::find()
            .filter(user::Column::Role.eq(Role::Owner))
            .filter(user::Column::Status.eq(Status::Active))
            .order_by_asc(user::Column::Id)
            .lock_exclusive()
            .all(&blocker)
            .await
            .unwrap();

        let edit_path = format!("/admin/api/v1/users/{}", creator.user_id);
        let edit = h.write(
            Method::PATCH,
            &edit_path,
            &editor,
            json!({ "display_name": "renamed" }),
        );
        let revoke = async {
            wait_for_lock_waiters(&h, 1).await;
            h.act(Method::POST, &revoke_path(token), &revoker).await
        };
        let release = async {
            wait_for_lock_waiters(&h, 2).await;
            blocker.commit().await.unwrap();
        };
        let both = async { tokio::join!(edit, revoke, release) };
        let (edited, revoked, ()) = tokio::time::timeout(Duration::from_secs(30), both)
            .await
            .expect("requests hung");

        assert_eq!(edited.status, 200, "{}", edited.body);
        assert_eq!(revoked.status, 200, "{}", revoked.body);
        assert_eq!(revoked.body["status"], "revoked");
    })
    .await;
}
