//! Password logins of directory accounts (06 §3.5) against the fake directory: routing
//! between local and LDAP accounts, service bind -> search -> user bind, provisioning,
//! no fallback to local passwords, outages, equal answers for unknown and wrong, and
//! filter escaping. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ldap::{Env, LOCAL_PASSWORD};
use common::ldap_fake::{FakeUser, Op, SERVICE_DN, USER_BASE, USER_PASSWORD, user_dn};
use deskhaven_core::ldap::DirectoryUpdate;
use deskhaven_core::users::{AuthError, AuthSource, Credential, NewUser, Role, Status};
use deskhaven_db::entity::user;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;

async fn row(db: &TestDb, id: i64) -> user::Model {
    user::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn first_login_provisions_a_directory_account_after_service_bind_search_and_user_bind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let dn = env.fake.add_user("alice");

        let done = env.login("alice", USER_PASSWORD).await.unwrap();

        let account = done.user;
        assert_eq!(account.username, "alice", "{}", db.backend);
        assert_eq!(account.auth_source, AuthSource::Ldap);
        assert_eq!(account.role, Role::User);
        assert_eq!(account.status, Status::Active);
        assert_eq!(account.display_name, "User alice");
        // Directory addresses are stored, never as verified.
        assert_eq!(account.email.as_deref(), Some("alice@example.org"));
        assert!(!account.email_verified);
        assert_eq!(row(db, account.id).await.password_hash, None);
        assert_eq!(env.fake.binds(), [SERVICE_DN.to_owned(), dn]);
        let searches = env.fake.searches();
        assert_eq!(searches.len(), 1);
        assert_eq!(searches[0].base, USER_BASE);
        assert_eq!(
            searches[0].filter,
            "(&(objectClass=inetOrgPerson)(uid=alice))"
        );
        assert!(env.fake.log().ends_with(&[Op::Close]));

        // The second login reaches the same account.
        let again = env.login("ALICE", USER_PASSWORD).await.unwrap();
        assert_eq!(again.user.id, account.id);
    })
    .await;
}

#[tokio::test]
async fn wrong_password_and_unknown_user_answer_the_same_and_both_bind_once() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");

        let wrong = env.login("alice", "not-the-password").await;
        let wrong_binds = env.fake.binds().len();
        env.fake.clear_log();
        let unknown = env.login("mallory", USER_PASSWORD).await;
        let unknown_binds = env.fake.binds();

        assert!(
            matches!(wrong, Err(AuthError::InvalidCredentials)),
            "{}: {wrong:?}",
            db.backend
        );
        assert!(
            matches!(unknown, Err(AuthError::InvalidCredentials)),
            "{unknown:?}"
        );
        // Service bind plus one password bind in both cases (a decoy DN for the unknown
        // user), so neither the answer nor the round trips tell them apart.
        assert_eq!(wrong_binds, 2);
        assert_eq!(unknown_binds.len(), 2);
        assert!(unknown_binds[1].ends_with(USER_BASE), "{unknown_binds:?}");
        assert_ne!(unknown_binds[1], user_dn("mallory"));
        // Nothing was provisioned.
        assert!(env.users.get_by_username("mallory").await.is_err());
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_directory_fails_the_login_and_never_falls_back() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        env.login("alice", USER_PASSWORD).await.unwrap();
        env.local_user("bob").await;
        env.fake.set_down(true);

        let existing = env.login("alice", USER_PASSWORD).await;
        let unknown = env.login("carol", USER_PASSWORD).await;
        let local = env.login("bob", LOCAL_PASSWORD).await;

        assert!(
            matches!(existing, Err(AuthError::DirectoryUnavailable)),
            "{}: {existing:?}",
            db.backend
        );
        // An unknown name answers like a wrong local password (no 503 vs 401 oracle).
        assert!(
            matches!(unknown, Err(AuthError::InvalidCredentials)),
            "{unknown:?}"
        );
        // Local accounts never depend on the directory.
        assert_eq!(local.unwrap().user.username, "bob");
    })
    .await;
}

#[tokio::test]
async fn a_wrong_service_password_is_an_outage_not_a_wrong_user_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| d.bind_password = Some("stale".to_owned())).await;
        env.fake.add_user("alice");
        env.ldap_user("alice").await;

        let outcome = env.login("alice", USER_PASSWORD).await;

        assert!(
            matches!(outcome, Err(AuthError::DirectoryUnavailable)),
            "{}: {outcome:?}",
            db.backend
        );
        assert_eq!(env.fake.binds(), [SERVICE_DN.to_owned()]);
    })
    .await;
}

#[tokio::test]
async fn local_and_oidc_accounts_never_reach_the_directory() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        // The directory knows both names, with the directory password.
        env.fake.add_user("bob");
        env.fake.add_user("olga");
        env.local_user("bob").await;
        env.users
            .create(NewUser::new("olga", Credential::Oidc))
            .await
            .unwrap();

        let local_with_directory_password = env.login("bob", USER_PASSWORD).await;
        let oidc = env.login("olga", USER_PASSWORD).await;
        let local = env.login("bob", LOCAL_PASSWORD).await;

        assert!(
            matches!(
                local_with_directory_password,
                Err(AuthError::InvalidCredentials)
            ),
            "{}",
            db.backend
        );
        assert!(
            matches!(oidc, Err(AuthError::InvalidCredentials)),
            "{oidc:?}"
        );
        assert!(local.is_ok());
        assert_eq!(env.fake.connects(), 0);
    })
    .await;
}

#[tokio::test]
async fn an_empty_password_is_refused_before_any_bind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");

        // The fake accepts empty passwords, like an unauthenticated bind would.
        let outcome = env.login("alice", "").await;

        assert!(
            matches!(outcome, Err(AuthError::InvalidCredentials)),
            "{}: {outcome:?}",
            db.backend
        );
        assert_eq!(env.fake.connects(), 0);
    })
    .await;
}

#[tokio::test]
async fn ambiguous_matches_and_mismatched_usernames_are_refused_without_a_user_bind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let first = env.fake.add_user("dup");
        let mut twin = FakeUser {
            dn: format!("uid=dup,ou=contractors,{USER_BASE}"),
            password: USER_PASSWORD.to_owned(),
            attrs: Default::default(),
        };
        twin.attrs.insert("uid".to_owned(), vec!["dup".to_owned()]);
        env.fake.put_user(twin);
        // A directory entry whose username attribute is not a valid deskhaven username.
        let odd = env.fake.add_user("odd");
        env.fake.set_attr(&odd, "uid", &["odd "]);

        let ambiguous = env.login("dup", USER_PASSWORD).await;
        let binds = env.fake.binds();
        let mismatched = env.login("odd", USER_PASSWORD).await;

        assert!(
            matches!(ambiguous, Err(AuthError::InvalidCredentials)),
            "{}: {ambiguous:?}",
            db.backend
        );
        assert!(!binds.contains(&first), "{binds:?}");
        assert!(
            matches!(mismatched, Err(AuthError::InvalidCredentials)),
            "{mismatched:?}"
        );
        assert!(env.users.get_by_username("odd").await.is_err());
    })
    .await;
}

#[tokio::test]
async fn disabled_and_unverified_accounts_are_told_only_after_the_right_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        let id = env.login("alice", USER_PASSWORD).await.unwrap().user.id;
        env.users.set_status(id, Status::Disabled).await.unwrap();

        let wrong = env.login("alice", "nope-nope").await;
        let right = env.login("alice", USER_PASSWORD).await;

        assert!(
            matches!(wrong, Err(AuthError::InvalidCredentials)),
            "{}",
            db.backend
        );
        assert!(matches!(right, Err(AuthError::Disabled)), "{right:?}");
    })
    .await;
}

#[tokio::test]
async fn a_disabled_directory_refuses_its_accounts_and_unknown_names_stay_local() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        env.login("alice", USER_PASSWORD).await.unwrap();
        let off = DirectoryUpdate {
            enabled: Some(false),
            ..DirectoryUpdate::default()
        };
        env.ldap
            .update_directory(env.directory.id, off)
            .await
            .unwrap();
        env.fake.clear_log();

        let existing = env.login("alice", USER_PASSWORD).await;
        let unknown = env.login("carol", USER_PASSWORD).await;

        assert!(
            matches!(existing, Err(AuthError::InvalidCredentials)),
            "{}: {existing:?}",
            db.backend
        );
        assert!(
            matches!(unknown, Err(AuthError::InvalidCredentials)),
            "{unknown:?}"
        );
        assert_eq!(env.fake.connects(), 0);
    })
    .await;
}

#[tokio::test]
async fn the_credential_stamp_is_the_directory_accounts_own() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        let done = env.login("alice", USER_PASSWORD).await.unwrap();

        let matches = env
            .users
            .credential_stamp_matches(&db.conn, done.user.id, &done.stamp)
            .await
            .unwrap();

        assert!(matches, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn provisioning_keeps_an_address_another_account_holds_off_the_new_one() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let mut bob = NewUser::new(
            "bob",
            Credential::Local {
                password: deskhaven_core::users::Password::new(LOCAL_PASSWORD),
            },
        );
        bob.email = Some("alice@example.org".to_owned());
        env.users.create(bob).await.unwrap();
        env.fake.add_user("alice");

        let alice = env.login("alice", USER_PASSWORD).await.unwrap().user;

        assert_eq!(alice.email, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn provisioned_display_names_lose_control_bidi_and_zero_width_characters() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let dn = env.fake.add_user("alice");
        env.fake
            .set_attr(&dn, "cn", &["Ali\u{202E}ce\u{200B} Lid\u{7}dell"]);

        let alice = env.login("alice", USER_PASSWORD).await.unwrap().user;

        assert_eq!(alice.display_name, "Alice Liddell", "{}", db.backend);
    })
    .await;
}
