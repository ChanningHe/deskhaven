//! Console sign-in of directory accounts (06 §3.5) through the real router and the fake
//! directory: the directory password signs in, wrong and unknown answer the same 401, an
//! unreachable directory is a 503 (directory accounts) that never falls back to a local
//! password, TOTP applies to directory accounts (enrolment re-authenticates against the
//! directory), login events say `ldap`. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use common::login::{console_of, harness_with_ldap};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::AuthSource;
use deskhaven_db::entity::login_event;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD, group_dn};
use sea_orm::EntityTrait;
use serde_json::json;

const ME: &str = "/admin/api/v1/me";
const ENROL: &str = "/admin/api/v1/me/totp/enrolment";
const CONFIRM: &str = "/admin/api/v1/me/totp/enrolment/confirm";
const TOTP_LOGIN: &str = "/admin/api/v1/auth/login/totp";

async fn harness(db: &TestDb, fake: &FakeDirectory) -> common::Harness {
    let h = harness_with_ldap(db, fake.connector()).await;
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    new.admin_group_dn = Some(group_dn("admins"));
    h.state.ldap.create_directory(new).await.unwrap();
    h
}

#[tokio::test]
async fn a_directory_account_signs_in_with_the_directory_password() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;

        let reply = h.login("alice", USER_PASSWORD).await;

        assert_eq!(reply.status, 200, "{}: {:?}", db.backend, reply.body);
        assert_eq!(reply.body["status"], "signed_in");
        let account = h.state.users.get_by_username("alice").await.unwrap();
        assert_eq!(account.auth_source, AuthSource::Ldap);
        assert_eq!(reply.body["user"]["id"], account.id);
    })
    .await;
}

#[tokio::test]
async fn wrong_and_unknown_get_the_same_answer() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;

        let wrong = h.login("alice", "not-the-password").await;
        let unknown = h.login("mallory", USER_PASSWORD).await;

        for reply in [&wrong, &unknown] {
            assert_eq!(reply.status, 401, "{}: {:?}", db.backend, reply.body);
            assert_eq!(reply.code(), "invalid_credentials");
        }
        assert_eq!(wrong.body, unknown.body);
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_directory_is_a_503_for_directory_accounts_only() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;
        assert_eq!(h.login("alice", USER_PASSWORD).await.status, 200);
        fake.set_down(true);

        let reply = h.login("alice", USER_PASSWORD).await;
        let unknown = h.login("mallory", USER_PASSWORD).await;

        assert_eq!(reply.status, 503, "{}: {:?}", db.backend, reply.body);
        assert_eq!(reply.code(), "unavailable");
        assert!(reply.headers.contains_key("retry-after"));
        // An unknown name answers like a wrong local password: no status oracle.
        assert_eq!(unknown.status, 401, "{:?}", unknown.body);
        assert_eq!(unknown.code(), "invalid_credentials");
    })
    .await;
}

#[tokio::test]
async fn enforced_2fa_makes_a_directory_admin_enrol_with_the_directory_password() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        let dn = fake.add_user("alice");
        fake.set_members(&group_dn("admins"), &[&dn]);
        let h = harness(db, &fake).await;
        h.set_setting("auth.tfa.enforce", json!("admins")).await;
        let b = db.backend;

        let signed_in = h.login("alice", USER_PASSWORD).await;
        assert_eq!(signed_in.status, 200, "{b}: {:?}", signed_in.body);
        let console = console_of(&signed_in);
        let me = h.get(ME, &console).await;
        assert_eq!(me.body["setup_required"], "totp_enrolment", "{b}");
        let begun = h.post(ENROL, Some(&console), json!({})).await;
        assert_eq!(begun.status, 200, "{b}: {:?}", begun.body);
        let secret = TotpSecret::from_base32(begun.body["secret"].as_str().unwrap()).unwrap();
        let code = totp::code_at(&secret, SystemClock.now_ms());
        let wrong = json!({ "current_password": "nope", "code": code });
        let wrong = h.post(CONFIRM, Some(&console), wrong).await;
        let right = json!({ "current_password": USER_PASSWORD, "code": code });
        let confirmed = h.post(CONFIRM, Some(&console), right).await;

        assert_eq!(wrong.status, 422, "{b}: {:?}", wrong.body);
        assert_eq!(wrong.code(), "wrong_password");
        assert_eq!(confirmed.status, 200, "{b}: {:?}", confirmed.body);
        assert_eq!(confirmed.body["user"]["totp_enabled"], true);
        let next = h.login("alice", USER_PASSWORD).await;
        assert_eq!(next.body["status"], "totp_required");
    })
    .await;
}

#[tokio::test]
async fn an_enrolled_directory_account_needs_the_code_and_completes_with_it() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;
        let first = h.login("alice", USER_PASSWORD).await;
        let id = first.body["user"]["id"].as_i64().unwrap();
        let totp = h.enrol(id).await;
        let b = db.backend;

        let step1 = h.login("alice", USER_PASSWORD).await;
        assert_eq!(
            step1.body["status"], "totp_required",
            "{b}: {:?}",
            step1.body
        );
        let challenge = step1.body["challenge"].as_str().unwrap();
        let code = totp.next_code();
        let body = json!({ "username": "alice", "challenge": challenge, "code": code });
        let step2 = h.post(TOTP_LOGIN, None, body).await;

        // The challenge carried the directory account's credential stamp.
        assert_eq!(step2.status, 200, "{b}: {:?}", step2.body);
        assert_eq!(step2.body["status"], "signed_in");
    })
    .await;
}

#[tokio::test]
async fn login_events_of_directory_accounts_say_ldap() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;

        h.login("alice", USER_PASSWORD).await;
        h.login("alice", "wrong-one").await;

        let events = login_event::Entity::find().all(&db.conn).await.unwrap();
        let methods: Vec<_> = events.iter().map(|e| e.method).collect();
        let ldap = login_event::Method::Ldap;
        assert_eq!(methods, [ldap, ldap], "{}", db.backend);
    })
    .await;
}
