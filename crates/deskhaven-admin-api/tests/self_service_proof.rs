//! "My account" (M3-13), the proof behind a change of the second factor: turning TOTP off
//! ends the user's other sessions, wrong passwords and wrong codes of regenerating the
//! recovery codes are counted by the login throttle, and a directory account proves with
//! the directory (a reachable one: positive; an unreachable one: 503, never counted).
//! Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use common::login::harness_with_ldap;
use common::own::{ACCOUNT, DISABLE, ME, RECOVERY, SESSIONS};
use common::{Console, PASSWORD, PEER, harness};
use deskhaven_admin_api::{AdminRoutes, SESSION_COOKIE, csrf_token};
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::users::{Password, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

async fn audit_rows(db: &TestDb, action: &str) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
}

fn proof(password: &str, code: &str) -> Value {
    json!({ "current_password": password, "code": code })
}

#[tokio::test]
async fn turning_totp_off_ends_the_other_sessions_and_records_how_many() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        // Another console session (minted directly: a login now asks for the code), warm in
        // the cache like the current one.
        let auth = h
            .state
            .users
            .authenticate_local("alice", &Password::new(PASSWORD))
            .await
            .unwrap();
        let (_, token) = h
            .state
            .sessions
            .create_console(alice_id, &auth.stamp, Some(PEER.ip()), Some("other"))
            .await
            .unwrap();
        let other = Console {
            user_id: alice_id,
            cookie: format!("{SESSION_COOKIE}={}", token.expose()),
            csrf: csrf_token(token.expose()),
        };
        let b = db.backend;
        assert_eq!(h.get(ME, &other).await.status, 200, "{b}");
        assert_eq!(h.get(ME, &enrolled.console).await.status, 200, "{b}");

        let done = h
            .post(
                DISABLE,
                Some(&enrolled.console),
                proof(PASSWORD, &enrolled.recovery[0]),
            )
            .await;

        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(
            h.get(ME, &enrolled.console).await.status,
            200,
            "{b}: this one stays"
        );
        assert_eq!(
            h.get(ME, &other).await.status,
            401,
            "{b}: a warm session is gone at once"
        );
        let sessions = h.get(SESSIONS, &enrolled.console).await;
        assert_eq!(
            sessions.body["items"].as_array().unwrap().len(),
            1,
            "{b}: only this one"
        );
        let rows = audit_rows(db, "session.revoke_others").await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(alice_id), "{b}");
        assert!(
            rows[0].after.as_deref().unwrap().contains("\"revoked\""),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn wrong_passwords_and_codes_of_regenerating_are_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let b = db.backend;
        for wrong_password in [true, false] {
            let h = harness(db, AdminRoutes::new()).await;
            let name = if wrong_password { "pw" } else { "cd" };
            h.user(name, Role::User).await;
            let console = h.signed_in(name).await;
            let enrolled = h.enrol_through_console(&console).await;
            let body = if wrong_password {
                proof("not it", &enrolled.next_code())
            } else {
                proof(PASSWORD, "000000")
            };
            let mut replies = Vec::new();
            for _ in 0..5 {
                replies.push(
                    h.post(RECOVERY, Some(&enrolled.console), body.clone())
                        .await,
                );
            }
            let locked = h.login(name, PASSWORD).await;

            let expected = if wrong_password {
                "wrong_password"
            } else {
                "invalid_code"
            };
            for reply in &replies[..4] {
                assert_eq!(reply.code(), expected, "{b}");
            }
            assert_eq!(locked.status, 429, "{b} {name}: {:?}", locked.body);
        }
    })
    .await;
}

async fn ldap_harness(db: &TestDb, fake: &FakeDirectory) -> common::Harness {
    let h = harness_with_ldap(db, fake.connector()).await;
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    h.state.ldap.create_directory(new).await.unwrap();
    h
}

#[tokio::test]
async fn a_directory_account_proves_with_the_directory_and_an_outage_is_not_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = ldap_harness(db, &fake).await;
        let signed_in = h.login("alice", USER_PASSWORD).await;
        assert_eq!(signed_in.status, 200, "{:?}", signed_in.body);
        let console = common::login::console_of(&signed_in);
        let enrolled = h.enrol_with_password(&console, USER_PASSWORD).await;
        let b = db.backend;

        // The local password does not exist; the directory's does.
        let wrong = h
            .post(
                DISABLE,
                Some(&enrolled.console),
                proof(PASSWORD, &enrolled.next_code()),
            )
            .await;
        fake.set_down(true);
        let mut down = Vec::new();
        for _ in 0..6 {
            let body = proof(USER_PASSWORD, &enrolled.next_code());
            down.push(h.post(DISABLE, Some(&enrolled.console), body).await);
        }
        fake.set_down(false);
        let done = h
            .post(
                DISABLE,
                Some(&enrolled.console),
                proof(USER_PASSWORD, &enrolled.next_code()),
            )
            .await;

        assert_eq!(wrong.code(), "wrong_password", "{b}: {:?}", wrong.body);
        for reply in &down {
            assert_eq!(reply.status, 503, "{b}: never 429: {:?}", reply.body);
            assert_eq!(reply.code(), "unavailable", "{b}");
        }
        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["totp_enabled"], false, "{b}");
        assert_eq!(
            h.get(ACCOUNT, &enrolled.console).await.body["totp_enabled"],
            false,
            "{b}"
        );
    })
    .await;
}
