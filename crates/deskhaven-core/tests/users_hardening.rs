//! UserService hardening: credential stamps against the login / password-change race,
//! bounded pagination, best-effort rehash, fail-closed on corrupt hashes. Every test runs on
//! SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::pagination::{MAX_PAGE, PageRequest};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    AuthError, Credential, HashParams, NewUser, Password, PasswordHasher, Status, UserError,
    UserQuery, UserService,
};
use deskhaven_db::entity::user;
use deskhaven_db::testing::{Backend, TestDb, each_migrated_backend};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

async fn service_with(db: &TestDb, params: HashParams) -> UserService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    UserService::new(
        db.conn.clone(),
        settings,
        PasswordHasher::new(params).unwrap(),
    )
}

fn pw(s: &str) -> Password {
    Password::new(s)
}

fn local(username: &str) -> NewUser {
    let password = pw(PASSWORD);
    NewUser::new(username, Credential::Local { password })
}

async fn stored(db: &TestDb, id: i64) -> user::Model {
    user::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn password_changed_at_is_set_for_local_passwords_only() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service_with(db, FAST).await;

        let local_user = svc.create(local("alice")).await.unwrap();
        let ldap_user = svc
            .create(NewUser::new("bob", Credential::Ldap))
            .await
            .unwrap();

        let first = stored(db, local_user.id).await.password_changed_at.unwrap();
        assert_eq!(stored(db, ldap_user.id).await.password_changed_at, None);
        svc.set_password(local_user.id, &pw("another-Phrase-7"), false)
            .await
            .unwrap();
        let second = stored(db, local_user.id).await.password_changed_at.unwrap();
        assert!(second.0 > first.0, "{second:?} <= {first:?}");
    })
    .await;
}

#[tokio::test]
async fn stamp_from_before_a_password_change_no_longer_matches() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service_with(db, FAST).await;
        let carol = svc.create(local("carol")).await.unwrap();

        // Login verified the old password ...
        let login = svc
            .authenticate_local("carol", &pw(PASSWORD))
            .await
            .unwrap();
        assert!(
            svc.credential_stamp_matches(&db.conn, carol.id, &login.stamp)
                .await
                .unwrap()
        );
        // ... then the password changes before the session is created.
        svc.set_password(carol.id, &pw("another-Phrase-7"), false)
            .await
            .unwrap();

        let stale = svc
            .credential_stamp_matches(&db.conn, carol.id, &login.stamp)
            .await
            .unwrap();
        assert!(!stale);
        let fresh = svc
            .authenticate_local("carol", &pw("another-Phrase-7"))
            .await
            .unwrap();
        assert!(
            svc.credential_stamp_matches(&db.conn, carol.id, &fresh.stamp)
                .await
                .unwrap()
        );
    })
    .await;
}

#[tokio::test]
async fn stamp_stops_matching_when_the_user_is_disabled_or_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service_with(db, FAST).await;
        let dave = svc.create(local("dave")).await.unwrap();
        let login = svc.authenticate_local("dave", &pw(PASSWORD)).await.unwrap();

        svc.set_status(dave.id, Status::Disabled).await.unwrap();
        let disabled = svc
            .credential_stamp_matches(&db.conn, dave.id, &login.stamp)
            .await
            .unwrap();
        svc.delete(dave.id).await.unwrap();
        let deleted = svc
            .credential_stamp_matches(&db.conn, dave.id, &login.stamp)
            .await
            .unwrap();

        assert!(!disabled);
        assert!(!deleted);
    })
    .await;
}

#[tokio::test]
async fn rehash_on_login_keeps_the_stamp() {
    each_migrated_backend(async |db: &TestDb| {
        let old = service_with(db, FAST).await;
        let erin = old.create(local("erin")).await.unwrap();
        let before = old.authenticate_local("erin", &pw(PASSWORD)).await.unwrap();

        let current = service_with(db, HashParams::DEFAULT).await;
        current
            .authenticate_local("erin", &pw(PASSWORD))
            .await
            .unwrap();

        assert!(
            current
                .credential_stamp_matches(&db.conn, erin.id, &before.stamp)
                .await
                .unwrap()
        );
    })
    .await;
}

#[tokio::test]
async fn huge_page_numbers_are_rejected_not_overflowed() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service_with(db, FAST).await;
        svc.create(local("frank")).await.unwrap();
        let query = |page| UserQuery {
            page: PageRequest {
                page,
                page_size: 100,
            },
            ..UserQuery::default()
        };

        for page in [100_000_000_000_000_000, u64::MAX, MAX_PAGE + 1] {
            let result = svc.list(&query(page)).await;
            assert!(
                matches!(result, Err(UserError::Invalid { field: "page", .. })),
                "{page}: {result:?}"
            );
        }
        let last_allowed = svc.list(&query(MAX_PAGE)).await.unwrap();
        assert!(last_allowed.items.is_empty());
        assert_eq!(last_allowed.total, 1);
    })
    .await;
}

/// Makes every UPDATE of `users.password_hash` fail.
async fn block_password_updates(db: &TestDb) {
    let statements: &[&str] = match db.backend {
        Backend::Sqlite => &[
            "CREATE TRIGGER block_rehash BEFORE UPDATE OF password_hash ON users \
             BEGIN SELECT RAISE(ABORT, 'blocked by test'); END",
        ],
        Backend::Postgres => &[
            "CREATE FUNCTION block_rehash() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'blocked by test'; END $$",
            "CREATE TRIGGER block_rehash BEFORE UPDATE OF password_hash ON users \
             FOR EACH ROW EXECUTE FUNCTION block_rehash()",
        ],
    };
    for sql in statements {
        db.conn.execute_unprepared(sql).await.unwrap();
    }
}

#[tokio::test]
async fn failed_rehash_still_authenticates() {
    each_migrated_backend(async |db: &TestDb| {
        let old = service_with(db, FAST).await;
        let grace = old.create(local("grace")).await.unwrap();
        let before = stored(db, grace.id).await.password_hash;
        block_password_updates(db).await;

        let current = service_with(db, HashParams::DEFAULT).await;
        let login = current.authenticate_local("grace", &pw(PASSWORD)).await;

        assert_eq!(login.unwrap().user.id, grace.id);
        assert_eq!(stored(db, grace.id).await.password_hash, before);
    })
    .await;
}

#[tokio::test]
async fn malformed_stored_hash_fails_closed_as_invalid_credentials() {
    each_migrated_backend(async |db: &TestDb| {
        let svc = service_with(db, FAST).await;
        let heidi = svc.create(local("heidi")).await.unwrap();
        user::Entity::update_many()
            .col_expr(user::Column::PasswordHash, Expr::value("$argon2id$garbage"))
            .filter(user::Column::Id.eq(heidi.id))
            .exec(&db.conn)
            .await
            .unwrap();

        let result = svc.authenticate_local("heidi", &pw(PASSWORD)).await;

        assert!(
            matches!(result, Err(AuthError::InvalidCredentials)),
            "{result:?}"
        );
    })
    .await;
}
