//! Console sign-in of directory accounts (06 §3.5) through the real router and the fake
//! directory: the directory password signs in, wrong and unknown answer the same 401, an
//! unreachable directory is a 503 that never falls back to a local password. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use common::login::harness_with_ldap;
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::users::AuthSource;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD};

async fn harness(db: &TestDb, fake: &FakeDirectory) -> common::Harness {
    let h = harness_with_ldap(db, fake.connector()).await;
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
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
async fn an_unreachable_directory_is_a_503() {
    each_migrated_backend(async |db: &TestDb| {
        let fake = FakeDirectory::new();
        fake.add_user("alice");
        let h = harness(db, &fake).await;
        assert_eq!(h.login("alice", USER_PASSWORD).await.status, 200);
        fake.set_down(true);

        let reply = h.login("alice", USER_PASSWORD).await;

        assert_eq!(reply.status, 503, "{}: {:?}", db.backend, reply.body);
        assert_eq!(reply.code(), "unavailable");
        assert!(reply.headers.contains_key("retry-after"));
    })
    .await;
}
