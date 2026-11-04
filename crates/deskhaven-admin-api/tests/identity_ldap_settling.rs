//! How LDAP "test user" settles the reserved login attempt (M3-09): counted when a login
//! would have counted (password sent, unknown name), refunded when the directory failed
//! before the user step; malformed usernames are refused before anything is reserved or
//! contacted; a timed-out test after the password was sent counts. Plus the audit of an
//! unsaved form. Runs on SQLite and PostgreSQL against the fake directory.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use std::time::Duration;

use axum::http::Method;
use common::login::harness_with_ldap_limits;
use common::tokens::snapshot;
use common::{Console, Harness, Reply};
use deskhaven_core::ldap::{DirectoryLimits, NewDirectory};
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD};
use serde_json::{Value, json};

const CONNECTION: &str = "/admin/api/v1/ldap-tests/connection";
const USER: &str = "/admin/api/v1/ldap-tests/user";

struct Env {
    h: Harness,
    fake: FakeDirectory,
    owner: Console,
    directory_id: i64,
}

async fn setup_with(db: &TestDb, limits: DirectoryLimits) -> Env {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let h = harness_with_ldap_limits(db, fake.connector(), limits).await;
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    let directory_id = h.state.ldap.create_directory(new).await.unwrap().id;
    let owner = h.console("own", Role::Owner).await;
    // One failure locks: the next test of the same name answers 429 if it was counted.
    h.set_setting("auth.lockout.max_failures", json!(1)).await;
    Env {
        h,
        fake,
        owner,
        directory_id,
    }
}

async fn setup(db: &TestDb) -> Env {
    setup_with(db, DirectoryLimits::default()).await
}

async fn test_user(env: &Env, target: Value, username: &str, password: &str) -> Reply {
    let mut body = json!({ "username": username, "password": password });
    if let (Some(body), Some(target)) = (body.as_object_mut(), target.as_object()) {
        body.extend(target.clone());
    }
    env.h.write(Method::POST, USER, &env.owner, body).await
}

fn saved(env: &Env) -> Value {
    json!({ "directory_id": env.directory_id })
}

/// The status of a test of `alice` after `first`: 429 when `first` was counted.
async fn next_status(env: &Env) -> u16 {
    env.fake.set_down(false);
    test_user(env, saved(env), "alice", "wrong-again")
        .await
        .status
        .as_u16()
}

async fn audit_row(h: &Harness, action: &str) -> console_audit::Model {
    for _ in 0..200 {
        if let Some(row) = h
            .audit_rows()
            .await
            .into_iter()
            .find(|r| r.action == action)
        {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no {action} audit row");
}

#[tokio::test]
async fn malformed_usernames_are_refused_before_anything_is_reserved_or_contacted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        env.fake.clear_log();
        let b = db.backend;

        for name in ["alice ", "ａｌｉｃｅ", "al ice", "alice\u{200b}"] {
            let reply = test_user(&env, saved(&env), name, "pw").await;
            assert_eq!(reply.status, 422, "{b}: {name:?} {:?}", reply.body);
            assert!(reply.body["error"]["details"]["fields"]["username"].is_array());
        }
        assert_eq!(env.fake.connects(), 0, "{b}");
        // Nothing was charged to the shared malformed row nor to alice.
        assert_eq!(next_status(&env).await, 200, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_directory_failing_before_the_user_step_refunds_the_attempt() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let b = db.backend;

        env.fake.set_down(true);
        let down = test_user(&env, saved(&env), "alice", "wrong-1").await;
        assert_eq!(down.status, 200, "{b}: {:?}", down.body);
        assert_eq!(down.body["steps"][0]["status"], "failed");
        env.fake.set_down(false);
        let form = json!({ "directory": {
            "name": "draft",
            "url": "ldaps://ldap.example.org",
            "bind_dn": SERVICE_DN,
            "bind_password": "not-the-service-password",
            "user_base_dn": USER_BASE,
        }});
        let refused = test_user(&env, form, "alice", "wrong-2").await;
        assert_eq!(
            refused.body["steps"][1]["status"], "failed",
            "{b}: {:?}",
            refused.body
        );

        assert_eq!(
            next_status(&env).await,
            200,
            "{b}: neither failure was counted"
        );
    })
    .await;
}

#[tokio::test]
async fn an_unknown_name_with_a_password_is_counted_like_a_login() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;

        let missing = test_user(&env, saved(&env), "nobody", "pw").await;
        let again = test_user(&env, saved(&env), "nobody", "pw").await;

        assert_eq!(missing.status, 200, "{}: {:?}", db.backend, missing.body);
        assert_eq!(missing.body["steps"][2]["status"], "failed");
        assert_eq!(again.status, 429, "{:?}", again.body);
    })
    .await;
}

#[tokio::test]
async fn a_test_timed_out_after_the_password_was_sent_is_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let limits = DirectoryLimits {
            diagnostic_timeout: Duration::from_millis(200),
            ..DirectoryLimits::default()
        };
        let env = setup_with(db, limits).await;
        env.fake.set_bind_delay(Some(Duration::from_secs(5)));

        let slow = test_user(&env, saved(&env), "alice", USER_PASSWORD).await;
        env.fake.set_bind_delay(None);

        assert_eq!(slow.status, 200, "{}: {:?}", db.backend, slow.body);
        assert_eq!(slow.body["steps"][3]["status"], "failed");
        let message = slow.body["steps"][3]["message"].as_str().unwrap();
        assert!(message.contains("timed out"), "{message}");
        assert_eq!(next_status(&env).await, 429);
    })
    .await;
}

#[tokio::test]
async fn an_unsaved_form_test_is_audited_with_its_connection() {
    each_migrated_backend(async |db: &TestDb| {
        let env = setup(db).await;
        let form = json!({ "directory": {
            "name": "draft",
            "url": "ldap://ldap.example.org",
            "start_tls": true,
            "tls_verify": false,
            "bind_dn": SERVICE_DN,
            "bind_password": SERVICE_PASSWORD,
            "user_base_dn": USER_BASE,
        }});

        let reply = env
            .h
            .write(Method::POST, CONNECTION, &env.owner, form)
            .await;
        assert_eq!(reply.status, 200, "{}: {:?}", db.backend, reply.body);

        let row = audit_row(&env.h, "ldap_directory.test_connection").await;
        let after = snapshot(&row.after);
        assert_eq!(row.target_id, "new");
        assert_eq!(after["url"], "ldap://ldap.example.org");
        assert_eq!(after["start_tls"], true);
        assert_eq!(after["tls_verify"], false);
        assert_eq!(after["bind_dn"], SERVICE_DN);
        let dump = format!("{:?}", row.after);
        assert!(!dump.contains(SERVICE_PASSWORD), "{dump}");
    })
    .await;
}
