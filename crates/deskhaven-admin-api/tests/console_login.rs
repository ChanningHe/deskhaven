//! Console login through the real router (M3-02): password step, TOTP / recovery code
//! step, equal answers for unknown and wrong, lockout, fixation, logout, login events and
//! the insecure-transport refusal. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::SocketAddr;

use axum::extract::ConnectInfo;
use axum::http::{Method, Request};
use common::login::{HOST, ORIGIN, console_of, set_cookies};
use common::{CSRF_HEADER_NAME, PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::{Role, Status};
use deskhaven_db::entity::{console_audit, login_event};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

const LOGIN: &str = "/admin/api/v1/auth/login";
const TOTP: &str = "/admin/api/v1/auth/login/totp";
const LOGOUT: &str = "/admin/api/v1/auth/logout";
const ME: &str = "/admin/api/v1/me";

async fn events(db: &TestDb) -> Vec<login_event::Model> {
    login_event::Entity::find()
        .order_by_asc(login_event::Column::Id)
        .all(&db.conn)
        .await
        .unwrap()
}

fn totp_body(username: &str, challenge: &str, code: &str) -> Value {
    json!({ "username": username, "challenge": challenge, "code": code })
}

#[tokio::test]
async fn a_password_login_sets_fresh_secure_cookies_and_records_the_event() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::Admin).await;
        let b = db.backend;

        let reply = h.login("Alice", PASSWORD).await;

        assert_eq!(reply.status, 200, "{b}: {:?}", reply.body);
        assert_eq!(reply.body["status"], "signed_in", "{b}");
        assert_eq!(reply.body["user"]["id"], id, "{b}");
        assert_eq!(reply.body["user"]["setup_required"], Value::Null, "{b}");
        let cookies = set_cookies(&reply);
        assert_eq!(cookies.len(), 2, "{b}: {cookies:?}");
        for cookie in &cookies {
            assert!(cookie.starts_with("__Host-deskhaven_"), "{cookie}");
            assert!(
                cookie.contains("; Secure") && cookie.contains("Path=/"),
                "{cookie}"
            );
        }
        assert!(
            cookies[0].contains("HttpOnly"),
            "{b}: session cookie is HttpOnly"
        );
        assert_eq!(reply.headers["cache-control"], "no-store", "{b}");
        let console = console_of(&reply);
        assert_eq!(h.get(ME, &console).await.status, 200, "{b}");
        let events = events(db).await;
        assert_eq!(events.len(), 1, "{b}");
        assert_eq!(events[0].result, "success", "{b}");
        assert_eq!(events[0].client, login_event::Client::Console, "{b}");
        assert_eq!(events[0].method, login_event::Method::Password, "{b}");
        assert_eq!(events[0].user_id, Some(id), "{b}");
    })
    .await;
}

#[tokio::test]
async fn unknown_accounts_and_wrong_passwords_get_the_same_answer() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let b = db.backend;

        let wrong = h.login("alice", "not the password").await;
        let unknown = h.login("nobody", PASSWORD).await;

        for reply in [&wrong, &unknown] {
            assert_eq!(reply.status, 401, "{b}");
            assert_eq!(reply.code(), "invalid_credentials", "{b}");
            assert!(set_cookies(reply).is_empty(), "{b}");
        }
        assert_eq!(wrong.body, unknown.body, "{b}");
        let events = events(db).await;
        assert_eq!(events.len(), 2, "{b}");
        for event in events {
            assert_eq!(event.result, "invalid_credentials", "{b}");
            assert_eq!(event.user_id, None, "{b}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_disabled_account_is_refused_only_with_the_right_password() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::User).await;
        h.state
            .users
            .set_status(id, Status::Disabled)
            .await
            .unwrap();
        let b = db.backend;

        let wrong = h.login("alice", "not the password").await;
        let right = h.login("alice", PASSWORD).await;

        assert_eq!(wrong.code(), "invalid_credentials", "{b}");
        assert_eq!(right.status, 403, "{b}");
        assert_eq!(right.code(), "account_disabled", "{b}");
    })
    .await;
}

#[tokio::test]
async fn repeated_failures_lock_the_username_even_for_the_right_password() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let b = db.backend;

        for _ in 0..5 {
            assert_eq!(h.login("alice", "wrong").await.status, 401, "{b}");
        }
        let locked = h.login("alice", PASSWORD).await;

        assert_eq!(locked.status, 429, "{b}: {:?}", locked.body);
        assert_eq!(locked.code(), "rate_limited", "{b}");
        assert!(locked.headers.contains_key("retry-after"), "{b}");
        let results: Vec<String> = events(db).await.into_iter().map(|e| e.result).collect();
        assert_eq!(
            results.last().map(String::as_str),
            Some("locked"),
            "{b}: {results:?}"
        );
        assert_eq!(results.len(), 5, "{b}: the refused attempt is not recorded");
    })
    .await;
}

#[tokio::test]
async fn a_totp_account_gets_a_challenge_without_a_cookie_then_signs_in_with_a_code() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::Owner).await;
        let totp = h.enrol(id).await;
        let b = db.backend;

        let step1 = h.login("alice", PASSWORD).await;
        assert_eq!(step1.status, 200, "{b}: {:?}", step1.body);
        assert_eq!(step1.body["status"], "totp_required", "{b}");
        assert!(set_cookies(&step1).is_empty(), "{b}: no session before 2FA");
        let challenge = step1.body["challenge"].as_str().unwrap().to_owned();

        let wrong = h
            .post(TOTP, None, totp_body("alice", &challenge, "000000"))
            .await;
        let code = totp.next_code();
        let right = h
            .post(TOTP, None, totp_body("alice", &challenge, &code))
            .await;
        let replay = h
            .post(TOTP, None, totp_body("alice", &challenge, &code))
            .await;

        assert_eq!(wrong.status, 422, "{b}: {:?}", wrong.body);
        assert_eq!(wrong.code(), "invalid_code", "{b}");
        assert_eq!(right.status, 200, "{b}: {:?}", right.body);
        assert_eq!(right.body["status"], "signed_in", "{b}");
        assert_eq!(h.get(ME, &console_of(&right)).await.status, 200, "{b}");
        assert_eq!(replay.status, 401, "{b}: the challenge is spent");
        assert_eq!(replay.code(), "invalid_challenge", "{b}");
        let results: Vec<(String, login_event::Method)> = events(db)
            .await
            .into_iter()
            .map(|e| (e.result, e.method))
            .collect();
        use login_event::Method::{Password, Totp};
        assert_eq!(
            results,
            [
                ("tfa_required".to_owned(), Password),
                ("invalid_code".to_owned(), Totp),
                ("success".to_owned(), Totp),
                ("invalid_challenge".to_owned(), Totp),
            ],
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_recovery_code_completes_the_second_step_once() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::User).await;
        let totp = h.enrol(id).await;
        let code = totp.recovery[0].clone();
        let b = db.backend;

        let mut replies = Vec::new();
        for _ in 0..2 {
            let step1 = h.login("alice", PASSWORD).await;
            let challenge = step1.body["challenge"].as_str().unwrap().to_owned();
            replies.push(
                h.post(TOTP, None, totp_body("alice", &challenge, &code))
                    .await,
            );
        }

        assert_eq!(replies[0].status, 200, "{b}: {:?}", replies[0].body);
        assert_eq!(replies[1].code(), "invalid_code", "{b}: used once");
        let methods: Vec<login_event::Method> = login_event::Entity::find()
            .filter(login_event::Column::Method.eq(login_event::Method::Recovery))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.method)
            .collect();
        assert_eq!(methods.len(), 2, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_challenge_works_only_for_its_username() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::User).await;
        h.user("mallory", Role::User).await;
        let totp = h.enrol(id).await;
        let b = db.backend;

        let step1 = h.login("alice", PASSWORD).await;
        let challenge = step1.body["challenge"].as_str().unwrap().to_owned();
        let other = h
            .post(
                TOTP,
                None,
                totp_body("mallory", &challenge, &totp.next_code()),
            )
            .await;

        assert_eq!(other.status, 401, "{b}");
        assert_eq!(other.code(), "invalid_challenge", "{b}");
    })
    .await;
}

#[tokio::test]
async fn signing_in_replaces_the_presented_session() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let old = h.signed_in("alice").await;
        let b = db.backend;

        let body = json!({ "username": "alice", "password": PASSWORD });
        let reply = h.post(LOGIN, Some(&old), body).await;

        let new = console_of(&reply);
        assert_ne!(new.cookie, old.cookie, "{b}");
        assert_eq!(
            h.get(ME, &old).await.status,
            401,
            "{b}: old session revoked"
        );
        assert_eq!(h.get(ME, &new).await.status, 200, "{b}");
    })
    .await;
}

#[tokio::test]
async fn logout_revokes_the_session_clears_the_cookies_and_is_audited() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let id = h.user("alice", Role::User).await;
        let console = h.signed_in("alice").await;
        let b = db.backend;

        let out = h.post(LOGOUT, Some(&console), json!({})).await;
        let again = h.post(LOGOUT, None, json!({})).await;

        assert_eq!(out.status, 204, "{b}: {:?}", out.body);
        let cleared = set_cookies(&out);
        for cookie in &cleared {
            assert!(cookie.contains("Max-Age=0"), "{b}: {cookie}");
        }
        let names: Vec<&str> = cleared.iter().filter_map(|c| c.split('=').next()).collect();
        assert_eq!(
            names,
            [
                "__Host-deskhaven_session",
                "__Host-deskhaven_csrf",
                "__Host-deskhaven_oidc"
            ],
            "{b}: a pending OIDC binding is cleared too"
        );
        assert_eq!(h.get(ME, &console).await.status, 401, "{b}");
        assert_eq!(again.status, 204, "{b}: idempotent");
        // The audit row is written by a detached task.
        let mut rows = Vec::new();
        for _ in 0..50 {
            rows = console_audit::Entity::find()
                .filter(console_audit::Column::Action.eq("session.logout"))
                .all(&db.conn)
                .await
                .unwrap();
            if !rows.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(id), "{b}");
    })
    .await;
}

#[tokio::test]
async fn logout_without_the_csrf_header_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let console = h.signed_in("alice").await;
        let b = db.backend;

        let headers = [
            ("origin", ORIGIN),
            ("host", HOST),
            ("cookie", console.cookie.as_str()),
        ];
        let reply = h
            .raw(common::request(
                Method::POST,
                LOGOUT,
                &headers,
                Some(json!({})),
            ))
            .await;

        assert_eq!(reply.status, 403, "{b}");
        assert_eq!(reply.code(), "csrf_failed", "{b}");
        assert_eq!(h.get(ME, &console).await.status, 200, "{b}");
    })
    .await;
}

#[tokio::test]
async fn plain_http_origins_are_refused_before_any_verification() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let b = db.backend;
        let body = json!({ "username": "alice", "password": PASSWORD });

        let lan = h
            .raw(common::request(
                Method::POST,
                LOGIN,
                &[
                    ("origin", "http://192.0.2.10:21114"),
                    ("host", "192.0.2.10:21114"),
                    (CSRF_HEADER_NAME, "anonymous"),
                ],
                Some(body.clone()),
            ))
            .await;
        let localhost = h
            .raw(common::request(
                Method::POST,
                LOGIN,
                &[
                    ("origin", "http://localhost:21114"),
                    ("host", "localhost:21114"),
                    (CSRF_HEADER_NAME, "anonymous"),
                ],
                Some(body.clone()),
            ))
            .await;

        assert_eq!(lan.status, 403, "{b}: {:?}", lan.body);
        assert_eq!(lan.code(), "insecure_transport", "{b}");
        assert!(set_cookies(&lan).is_empty(), "{b}");
        assert_eq!(localhost.status, 200, "{b}: {:?}", localhost.body);
        assert_eq!(events(db).await.len(), 1, "{b}: only the localhost login");
    })
    .await;
}

#[tokio::test]
async fn without_an_origin_only_a_loopback_peer_counts_as_secure_here() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let b = db.backend;
        let body = json!({ "username": "alice", "password": PASSWORD });

        let remote = h
            .raw(common::request(
                Method::POST,
                LOGIN,
                &[(CSRF_HEADER_NAME, "anonymous")],
                Some(body.clone()),
            ))
            .await;
        let mut tunnel: Request<_> = common::request(
            Method::POST,
            LOGIN,
            &[(CSRF_HEADER_NAME, "anonymous")],
            Some(body),
        );
        let loopback: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        tunnel.extensions_mut().insert(ConnectInfo(loopback));
        let tunnel = h.raw(tunnel).await;

        assert_eq!(remote.code(), "insecure_transport", "{b}");
        assert_eq!(tunnel.status, 200, "{b}: {:?}", tunnel.body);
    })
    .await;
}
