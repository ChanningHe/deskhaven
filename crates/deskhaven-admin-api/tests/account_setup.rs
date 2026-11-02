//! Sessions with an unfinished account setup (M3-02): restricted at the gate (not by the
//! SPA) to `/me`, the password change, the TOTP enrolment and logout; finishing a step
//! mints a fresh session and retires the restricted one. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::login::{console_of, set_cookies};
use common::{Console, Harness, PASSWORD, harness, probe};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::{ProfileUpdate, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

const ME: &str = "/admin/api/v1/me";
const PASSWORD_CHANGE: &str = "/admin/api/v1/me/password";
const ENROL: &str = "/admin/api/v1/me/totp/enrolment";
const CONFIRM: &str = "/admin/api/v1/me/totp/enrolment/confirm";
const NEW_PASSWORD: &str = "glacier Pen 1984 river";

/// Status (and code) of every kind of route for `console`.
async fn matrix(h: &Harness, console: &Console) -> Vec<(&'static str, u16, String)> {
    let mut out = Vec::new();
    let checks: [(&str, Method, &str); 5] = [
        ("session", Method::GET, ME),
        ("self", Method::GET, "/admin/api/v1/probe/split"),
        ("admin", Method::GET, "/admin/api/v1/probe/secret"),
        ("owner", Method::DELETE, "/admin/api/v1/probe/owner"),
        ("public", Method::POST, "/admin/api/v1/probe/public"),
    ];
    for (name, method, path) in checks {
        let reply = if method == Method::GET {
            h.get(path, console).await
        } else {
            h.write(method, path, console, json!({})).await
        };
        out.push((name, reply.status.as_u16(), reply.code().to_owned()));
    }
    out
}

fn restricted() -> Vec<(&'static str, u16, String)> {
    vec![
        ("session", 200, String::new()),
        ("self", 403, "setup_required".to_owned()),
        ("admin", 403, "setup_required".to_owned()),
        ("owner", 403, "setup_required".to_owned()),
        ("public", 200, String::new()),
    ]
}

fn unrestricted() -> Vec<(&'static str, u16, String)> {
    ["session", "self", "admin", "owner", "public"]
        .into_iter()
        .map(|name| (name, 200, String::new()))
        .collect()
}

async fn must_change(h: &Harness, username: &str) -> i64 {
    let id = h.user(username, Role::Owner).await;
    let update = ProfileUpdate {
        must_change_password: Some(true),
        ..ProfileUpdate::default()
    };
    h.state.users.update_profile(id, update).await.unwrap();
    id
}

async fn audit_rows(db: &TestDb, action: &str) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_temporary_password_restricts_the_session_at_the_gate() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, probe::routes()).await;
        must_change(&h, "alice").await;
        let b = db.backend;

        let console = h.signed_in("alice").await;
        let me = h.get(ME, &console).await;
        let gated = h.get("/admin/api/v1/probe/split", &console).await;

        assert_eq!(me.body["setup_required"], "password_change", "{b}");
        assert_eq!(matrix(&h, &console).await, restricted(), "{b}");
        assert_eq!(
            gated.body["error"]["details"]["step"], "password_change",
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_password_change_needs_the_current_password_and_mints_a_fresh_session() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, probe::routes()).await;
        let id = must_change(&h, "alice").await;
        let old = h.signed_in("alice").await;
        let other = h.signed_in("alice").await;
        let b = db.backend;

        let wrong = h
            .post(
                PASSWORD_CHANGE,
                Some(&old),
                json!({ "current_password": "nope", "new_password": NEW_PASSWORD }),
            )
            .await;
        let weak = h
            .post(
                PASSWORD_CHANGE,
                Some(&old),
                json!({ "current_password": PASSWORD, "new_password": "short" }),
            )
            .await;
        let changed = h
            .post(
                PASSWORD_CHANGE,
                Some(&old),
                json!({ "current_password": PASSWORD, "new_password": NEW_PASSWORD }),
            )
            .await;

        assert_eq!(wrong.status, 422, "{b}: never 401");
        assert_eq!(wrong.code(), "wrong_password", "{b}");
        assert_eq!(weak.status, 422, "{b}");
        assert_eq!(weak.code(), "validation_failed", "{b}");
        assert!(
            weak.body["error"]["details"]["fields"]["new_password"].is_array(),
            "{b}"
        );
        assert_eq!(changed.status, 200, "{b}: {:?}", changed.body);
        assert_eq!(changed.body["setup_required"], Value::Null, "{b}");
        let fresh = console_of(&changed);
        assert_ne!(fresh.cookie, old.cookie, "{b}");
        for revoked in [&old, &other] {
            assert_eq!(
                h.get(ME, revoked).await.status,
                401,
                "{b}: every old session revoked"
            );
        }
        assert_eq!(matrix(&h, &fresh).await, unrestricted(), "{b}");
        assert_eq!(h.login("alice", NEW_PASSWORD).await.status, 200, "{b}");
        let rows = audit_rows(db, "user.password_change").await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(id), "{b}");
        assert_eq!(rows[0].target_id, id.to_string(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn enforced_2fa_restricts_until_enrolment_which_rotates_every_console_session() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, probe::routes()).await;
        h.set_setting("auth.tfa.enforce", json!("all")).await;
        h.user("alice", Role::Owner).await;
        let console = h.signed_in("alice").await;
        let second = h.signed_in("alice").await;
        let b = db.backend;

        assert_eq!(
            h.get(ME, &console).await.body["setup_required"],
            "totp_enrolment",
            "{b}"
        );
        assert_eq!(matrix(&h, &console).await, restricted(), "{b}");

        let begun = h.post(ENROL, Some(&console), json!({})).await;
        assert_eq!(begun.status, 200, "{b}: {:?}", begun.body);
        assert_eq!(begun.headers["cache-control"], "no-store", "{b}");
        let secret = TotpSecret::from_base32(begun.body["secret"].as_str().unwrap()).unwrap();
        assert!(
            begun.body["uri"]
                .as_str()
                .unwrap()
                .starts_with("otpauth://totp/"),
            "{b}"
        );
        let size = begun.body["qr"]["size"].as_i64().unwrap();
        assert!(size >= 29, "{b}: {size}");
        assert!(
            begun.body["qr"]["path"]
                .as_str()
                .unwrap()
                .starts_with("M4 4h7"),
            "{b}"
        );

        let wrong = h
            .post(CONFIRM, Some(&console), json!({ "code": "000000" }))
            .await;
        let code = totp::code_at(&secret, SystemClock.now_ms());
        let confirmed = h
            .post(CONFIRM, Some(&console), json!({ "code": code }))
            .await;

        assert_eq!(wrong.status, 422, "{b}: never 401");
        assert_eq!(wrong.code(), "invalid_code", "{b}");
        assert_eq!(confirmed.status, 200, "{b}: {:?}", confirmed.body);
        let codes = confirmed.body["recovery_codes"].as_array().unwrap();
        assert_eq!(codes.len(), 10, "{b}");
        assert_eq!(confirmed.body["user"]["totp_enabled"], true, "{b}");
        assert_eq!(confirmed.body["user"]["setup_required"], Value::Null, "{b}");
        let rotated = console_of(&confirmed);
        for old in [&console, &second] {
            assert_eq!(h.get(ME, old).await.status, 401, "{b}: rotated away");
        }
        assert_eq!(matrix(&h, &rotated).await, unrestricted(), "{b}");
        assert_eq!(
            audit_rows(db, "user.totp_enrolment_begin").await.len(),
            1,
            "{b}"
        );
        assert_eq!(audit_rows(db, "user.totp_enable").await.len(), 1, "{b}");
        // The next login needs the code.
        assert_eq!(
            h.login("alice", PASSWORD).await.body["status"],
            "totp_required",
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn enforcement_applies_to_live_sessions_and_lifts_with_the_setting() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, probe::routes()).await;
        h.user("alice", Role::Admin).await;
        h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        let bob = h.signed_in("bob").await;
        let b = db.backend;

        h.set_setting("auth.tfa.enforce", json!("admins")).await;
        let alice_gated = h.get("/admin/api/v1/probe/split", &alice).await;
        let bob_open = h.get("/admin/api/v1/probe/split", &bob).await;
        h.set_setting("auth.tfa.enforce", json!("off")).await;
        let alice_open = h.get("/admin/api/v1/probe/split", &alice).await;

        assert_eq!(alice_gated.code(), "setup_required", "{b}");
        assert_eq!(
            bob_open.status, 200,
            "{b}: users are not covered by `admins`"
        );
        assert_eq!(alice_open.status, 200, "{b}");
    })
    .await;
}

#[tokio::test]
async fn enrolment_while_enabled_is_a_conflict_and_confirm_needs_a_pending_seed() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, probe::routes()).await;
        let id = h.user("alice", Role::User).await;
        let console = h.signed_in("alice").await;
        let b = db.backend;

        let early = h
            .post(CONFIRM, Some(&console), json!({ "code": "123456" }))
            .await;
        h.enrol(id).await;
        let again = h.post(ENROL, Some(&console), json!({})).await;

        assert_eq!(early.status, 409, "{b}: {:?}", early.body);
        assert_eq!(again.status, 409, "{b}");
        assert!(set_cookies(&again).is_empty(), "{b}");
    })
    .await;
}
