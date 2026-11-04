//! LDAP logins against a real directory through the production ldap3 connector. Skipped
//! unless `DESKHAVEN_TEST_LDAP_URL` names a directory seeded with
//! `tests/fixtures/ldap/seed.ldif`; `DESKHAVEN_TEST_LDAPS_URL` (its LDAPS port, with the
//! image's self-signed certificate) adds the certificate checks. Locally:
//!
//! ```sh
//! docker run -d --name deskhaven-ldap -p 127.0.0.1:3389:389 -p 127.0.0.1:3636:636 \
//!   -e LDAP_ADMIN_PASSWORD=admin-Secret-9 -e LDAP_TLS_VERIFY_CLIENT=never \
//!   -v "$PWD/crates/deskhaven-core/tests/fixtures/ldap:/container/service/slapd/assets/config/bootstrap/ldif/custom" \
//!   osixia/openldap:1.5.0 --copy-service
//! DESKHAVEN_TEST_LDAP_URL=ldap://127.0.0.1:3389 DESKHAVEN_TEST_LDAPS_URL=ldaps://127.0.0.1:3636 \
//!   cargo test -p deskhaven-core --test ldap_server
//! ```

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::ldap::service;
use common::settings;
use deskhaven_core::groups::{GroupService, NewGroup};
use deskhaven_core::ldap::{Ldap3Connector, LdapService, NewDirectory, Step, StepStatus};
use deskhaven_core::users::{
    AuthError, AuthSource, Credential, HashParams, NewUser, Password, PasswordHasher, Role,
    UserService,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

const URL_ENV: &str = "DESKHAVEN_TEST_LDAP_URL";
const LDAPS_URL_ENV: &str = "DESKHAVEN_TEST_LDAPS_URL";
const ADMIN_DN: &str = "cn=admin,dc=example,dc=org";
const ADMIN_PASSWORD: &str = "admin-Secret-9";
const ALICE_PASSWORD: &str = "alice-Dir-Pass-1";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

fn url(var: &str) -> Option<String> {
    let url = std::env::var(var).ok().filter(|u| !u.is_empty());
    if url.is_none() {
        eprintln!("{var} is not set: skipping the real-directory LDAP test");
    }
    url
}

fn directory(url: &str) -> NewDirectory {
    let mut new = NewDirectory::new("seed", url, "ou=people,dc=example,dc=org");
    new.bind_dn = Some(ADMIN_DN.to_owned());
    new.bind_password = Some(ADMIN_PASSWORD.to_owned());
    new.user_filter = "(objectClass=inetOrgPerson)".to_owned();
    new.email_attr = Some("mail".to_owned());
    new.display_name_attr = Some("cn".to_owned());
    new.group_base_dn = Some("ou=groups,dc=example,dc=org".to_owned());
    new.admin_group_dn = Some("cn=deskhaven-admins,ou=groups,dc=example,dc=org".to_owned());
    new
}

struct Env {
    ldap: LdapService,
    groups: GroupService,
    users: UserService,
}

impl Env {
    /// The directory account alice, as an earlier login left it.
    async fn alice(&self) {
        let new = NewUser::new("alice", Credential::Ldap);
        self.users.create(new).await.unwrap();
    }
}

async fn env(db: &TestDb, new: NewDirectory) -> Env {
    let hasher = PasswordHasher::new(FAST).unwrap();
    let users = UserService::new(db.conn.clone(), settings(db).await, hasher);
    let groups = GroupService::new(db.conn.clone());
    let ldap = service(db, &users, &groups, Arc::new(Ldap3Connector::new()));
    ldap.create_directory(new).await.unwrap();
    Env {
        ldap,
        groups,
        users,
    }
}

async fn login(
    env: &Env,
    username: &str,
    password: &str,
) -> Result<deskhaven_core::users::User, AuthError> {
    let done = env
        .ldap
        .authenticate_password(username, &Password::new(password))
        .await?;
    Ok(done.user)
}

#[tokio::test]
async fn logins_provision_map_groups_and_refuse_wrong_passwords() {
    let Some(url) = url(URL_ENV) else { return };
    each_migrated_backend(async |db: &TestDb| {
        for group_filter in [None, Some("(uniqueMember={user_dn})")] {
            let mut new = directory(&url);
            new.group_filter = group_filter.map(str::to_owned);
            let env = env(db, new).await;
            if env.groups.list().await.unwrap().is_empty() {
                env.groups.create(NewGroup::new("ops")).await.unwrap();
            }

            let alice = login(&env, "alice", ALICE_PASSWORD).await.unwrap();
            let wrong = login(&env, "alice", "not-her-password").await;
            let unknown = login(&env, "nobody", ALICE_PASSWORD).await;
            let bob = login(&env, "bob", "bob-Dir-Pass-2").await.unwrap();

            assert_eq!(alice.auth_source, AuthSource::Ldap, "{}", db.backend);
            assert_eq!(alice.display_name, "Alice Liddell");
            assert_eq!(alice.email.as_deref(), Some("alice@example.org"));
            assert_eq!(alice.role, Role::Admin);
            let groups = env.groups.groups_of_user(alice.id).await.unwrap();
            assert_eq!(groups.len(), 1, "{group_filter:?}");
            assert_eq!(groups[0].name, "ops");
            assert!(
                matches!(wrong, Err(AuthError::InvalidCredentials)),
                "{wrong:?}"
            );
            assert!(
                matches!(unknown, Err(AuthError::InvalidCredentials)),
                "{unknown:?}"
            );
            assert_eq!(bob.role, Role::User);
            assert!(env.groups.groups_of_user(bob.id).await.unwrap().is_empty());
            env.ldap
                .delete_directory(env.ldap.list_directories().await.unwrap()[0].id)
                .await
                .unwrap();
        }
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_directory_fails_the_login() {
    // Needs no directory: runs everywhere.
    each_migrated_backend(async |db: &TestDb| {
        // Nothing listens on port 1 of the loopback host.
        let env = env(db, directory("ldap://127.0.0.1:1")).await;
        env.alice().await;

        let outcome = login(&env, "alice", ALICE_PASSWORD).await;

        assert!(
            matches!(outcome, Err(AuthError::DirectoryUnavailable)),
            "{}: {outcome:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn diagnostics_run_against_the_directory() {
    let Some(url) = url(URL_ENV) else { return };
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db, directory(&url)).await;
        let id = env.ldap.list_directories().await.unwrap()[0].id;
        let settings = env.ldap.stored_settings(id).await.unwrap();
        let password = Password::new(ALICE_PASSWORD);

        let connection = env.ldap.test_connection(&settings).await;
        let user = env
            .ldap
            .test_user(&settings, "alice", Some(&password))
            .await;

        assert!(connection.passed(), "{}: {connection:?}", db.backend);
        assert!(user.passed(), "{user:?}");
        let probed = user.user.unwrap();
        assert_eq!(probed.dn, "uid=alice,ou=people,dc=example,dc=org");
        assert_eq!(probed.admin, Some(true));
        assert_eq!(probed.directory_groups.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn ldaps_verifies_the_certificate_unless_told_not_to() {
    let Some(url) = url(LDAPS_URL_ENV) else {
        return;
    };
    each_migrated_backend(async |db: &TestDb| {
        let verified = env(db, directory(&url)).await;
        let id = verified.ldap.list_directories().await.unwrap()[0].id;
        let settings = verified.ldap.stored_settings(id).await.unwrap();

        let refused = verified.ldap.test_connection(&settings).await;
        verified.alice().await;
        let login_refused = login(&verified, "alice", ALICE_PASSWORD).await;
        verified.ldap.delete_directory(id).await.unwrap();
        let mut unchecked = directory(&url);
        unchecked.tls_verify = false;
        let unchecked = env(db, unchecked).await;
        let id = unchecked.ldap.list_directories().await.unwrap()[0].id;
        let settings = unchecked.ldap.stored_settings(id).await.unwrap();
        let accepted = unchecked.ldap.test_connection(&settings).await;

        // The image's certificate is self-signed: refused by default.
        assert_eq!(refused.steps[0].step, Step::Connect, "{}", db.backend);
        assert_eq!(refused.steps[0].status, StepStatus::Failed, "{refused:?}");
        assert!(
            refused.steps[0].message.contains("certificate"),
            "{refused:?}"
        );
        assert!(matches!(
            login_refused,
            Err(AuthError::DirectoryUnavailable)
        ));
        assert!(accepted.passed(), "{accepted:?}");
    })
    .await;
}

#[tokio::test]
async fn start_tls_upgrades_the_plain_port_and_verifies_too() {
    let Some(url) = url(URL_ENV) else { return };
    each_migrated_backend(async |db: &TestDb| {
        let mut verified = directory(&url);
        verified.start_tls = true;
        let verified = env(db, verified).await;
        let id = verified.ldap.list_directories().await.unwrap()[0].id;
        let settings = verified.ldap.stored_settings(id).await.unwrap();
        let refused = verified.ldap.test_connection(&settings).await;
        verified.ldap.delete_directory(id).await.unwrap();
        let mut unchecked = directory(&url);
        unchecked.start_tls = true;
        unchecked.tls_verify = false;
        let unchecked = env(db, unchecked).await;

        let alice = login(&unchecked, "alice", ALICE_PASSWORD).await;

        assert_eq!(
            refused.steps[0].status,
            StepStatus::Failed,
            "{}: {refused:?}",
            db.backend
        );
        assert!(
            refused.steps[0].message.contains("certificate"),
            "{refused:?}"
        );
        assert_eq!(alice.unwrap().username, "alice");
    })
    .await;
}
