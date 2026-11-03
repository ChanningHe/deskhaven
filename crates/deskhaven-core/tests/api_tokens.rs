//! ApiTokenService (M2-19, 06 §3.2): `dhk_` tokens from the CSPRNG, only their SHA-256 stored,
//! scopes, expiry, revocation, read-time creator checks and the throttled `last_used_at`.
//! Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::PinSet;
use deskhaven_core::api_tokens::{
    API_TOKEN_PREFIX, ApiTokenError, ApiTokenService, NewApiToken, Scope,
};
use deskhaven_core::clock::{Clock, ManualClock};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, Role, Status, UserService,
};
use deskhaven_db::entity::api_token;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{DatabaseConnection, EntityTrait};
use sha2::{Digest, Sha256};

const T0_MS: i64 = 1_700_000_000_123;
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};
const MINUTE: Duration = Duration::from_secs(60);

struct Env {
    users: UserService,
    tokens: ApiTokenService,
    clock: Arc<ManualClock>,
    db: DatabaseConnection,
}

impl Env {
    async fn new(db: &TestDb) -> Self {
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
        let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
            .await
            .unwrap();
        let clock = Arc::new(ManualClock::at(T0_MS));
        let hasher = PasswordHasher::new(FAST).unwrap();
        let users = UserService::with_clock(db.conn.clone(), settings, hasher, clock.clone());
        let tokens = ApiTokenService::new(db.conn.clone(), clock.clone());
        Self {
            users,
            tokens,
            clock,
            db: db.conn.clone(),
        }
    }

    async fn admin(&self, name: &str) -> i64 {
        let password = Password::new("tundra-Lamp-42");
        let new = NewUser {
            role: Role::Admin,
            ..NewUser::new(name, Credential::Local { password })
        };
        self.users.create(new).await.unwrap().id
    }

    fn new_token(&self, created_by: Option<i64>, scopes: &[Scope]) -> NewApiToken {
        NewApiToken {
            name: "deploy script".to_owned(),
            scopes: scopes.iter().copied().collect(),
            created_by,
            expires_at: None,
        }
    }

    async fn issue(&self, created_by: Option<i64>, scopes: &[Scope]) -> (i64, String) {
        let (token, secret) = self
            .tokens
            .create(self.new_token(created_by, scopes))
            .await
            .unwrap();
        (token.id, secret.expose().to_owned())
    }

    async fn row(&self, id: i64) -> api_token::Model {
        api_token::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .unwrap()
            .unwrap()
    }
}

fn sha256_hex(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

#[tokio::test]
async fn create_returns_a_dhk_token_once_and_stores_only_its_hash() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("root").await;

        // Act
        let (token, secret) = env
            .tokens
            .create(env.new_token(Some(admin), &[Scope::Deploy, Scope::Assign]))
            .await
            .unwrap();

        // Assert
        let raw = secret.expose();
        assert!(raw.starts_with(API_TOKEN_PREFIX), "{}", db.backend);
        assert_eq!(raw.len(), API_TOKEN_PREFIX.len() + 43, "{}", db.backend);
        let row = env.row(token.id).await;
        assert_eq!(row.token_hash, sha256_hex(raw), "{}", db.backend);
        assert!(raw.starts_with(&row.token_prefix), "{}", db.backend);
        assert!(row.token_prefix.len() < raw.len() / 2, "{}", db.backend);
        assert_eq!(row.scopes, r#"["deploy","assign"]"#, "{}", db.backend);
        assert_eq!(row.created_by, Some(admin), "{}", db.backend);
        assert_eq!(row.created_at, time::from_millis(T0_MS).unwrap());
        assert!(
            !format!("{secret:?} {token:?}").contains(raw),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn create_rejects_blank_names_empty_scopes_and_past_expiry() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let past = time::from_millis(T0_MS - 1).unwrap();
        let cases = [
            NewApiToken {
                name: "  ".to_owned(),
                ..env.new_token(None, &[Scope::Deploy])
            },
            NewApiToken {
                name: "x".repeat(200),
                ..env.new_token(None, &[Scope::Deploy])
            },
            env.new_token(None, &[]),
            NewApiToken {
                expires_at: Some(past),
                ..env.new_token(None, &[Scope::Deploy])
            },
        ];

        for new in cases {
            let result = env.tokens.create(new).await;
            assert!(
                matches!(result, Err(ApiTokenError::Invalid { .. })),
                "{}: {result:?}",
                db.backend
            );
        }
        assert!(
            env.tokens.list().await.unwrap().is_empty(),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn create_for_an_unknown_creator_fails() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let result = env
            .tokens
            .create(env.new_token(Some(4242), &[Scope::Deploy]))
            .await;

        assert!(
            matches!(result, Err(ApiTokenError::CreatorNotFound)),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_authenticates_for_its_scopes_only() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("root").await;
        let (id, raw) = env.issue(Some(admin), &[Scope::Assign]).await;

        // Act
        let granted = env.tokens.authenticate(&raw, Scope::Assign).await.unwrap();
        let denied = env.tokens.authenticate(&raw, Scope::Deploy).await;

        // Assert
        assert_eq!(granted.token_id, id, "{}", db.backend);
        assert_eq!(granted.created_by, Some(admin), "{}", db.backend);
        assert_eq!(
            granted.scopes,
            BTreeSet::from([Scope::Assign]),
            "{}",
            db.backend
        );
        let err = denied.unwrap_err();
        assert!(matches!(err, ApiTokenError::MissingScope), "{}", db.backend);
        assert!(err.is_unauthenticated(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn unknown_and_malformed_tokens_do_not_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, raw) = env.issue(None, &[Scope::Deploy]).await;
        let mut flipped = raw.clone();
        let last = if flipped.ends_with('A') { "B" } else { "A" };
        flipped.replace_range(flipped.len() - 1.., last);

        for bad in [
            String::new(),
            "dhk_".to_owned(),
            raw[API_TOKEN_PREFIX.len()..].to_owned(),
            format!("{raw}A"),
            raw.replacen("dhk_", "dhx_", 1),
            flipped,
        ] {
            let err = env
                .tokens
                .authenticate(&bad, Scope::Deploy)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ApiTokenError::InvalidToken),
                "{}: {bad:?} {err:?}",
                db.backend
            );
            assert!(err.is_unauthenticated(), "{}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn expired_tokens_do_not_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let new = NewApiToken {
            expires_at: Some(time::from_millis(T0_MS + 60_000).unwrap()),
            ..env.new_token(None, &[Scope::Deploy])
        };
        let (_, secret) = env.tokens.create(new).await.unwrap();

        env.tokens
            .authenticate(secret.expose(), Scope::Deploy)
            .await
            .unwrap();
        env.clock.advance(MINUTE);
        let late = env
            .tokens
            .authenticate(secret.expose(), Scope::Deploy)
            .await;

        assert!(
            matches!(late, Err(ApiTokenError::Expired)),
            "{}: {late:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn revoked_tokens_do_not_authenticate_and_revoke_is_idempotent() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let (id, raw) = env.issue(None, &[Scope::Deploy]).await;

        // Act
        let first = env.tokens.revoke(id).await.unwrap();
        env.clock.advance(MINUTE);
        let again = env.tokens.revoke(id).await.unwrap();
        let unknown = env.tokens.revoke(id + 100).await.unwrap();
        let result = env.tokens.authenticate(&raw, Scope::Deploy).await;

        // Assert
        assert!(first && again && !unknown, "{}", db.backend);
        assert_eq!(
            env.row(id).await.revoked_at,
            Some(time::from_millis(T0_MS).unwrap()),
            "{}: the first revocation time is kept",
            db.backend
        );
        assert!(
            matches!(result, Err(ApiTokenError::Revoked)),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_dies_with_its_creator_or_their_admin_rights() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let disabled = env.admin("dora").await;
        let demoted = env.admin("dean").await;
        let deleted = env.admin("dell").await;
        let (_, t_disabled) = env.issue(Some(disabled), &[Scope::Deploy]).await;
        let (_, t_demoted) = env.issue(Some(demoted), &[Scope::Deploy]).await;
        let (id_deleted, _) = env.issue(Some(deleted), &[Scope::Deploy]).await;

        env.users
            .set_status(disabled, Status::Disabled)
            .await
            .unwrap();
        env.users.set_role(demoted, Role::User).await.unwrap();
        env.users.delete(deleted).await.unwrap();

        for raw in [t_disabled, t_demoted] {
            let result = env.tokens.authenticate(&raw, Scope::Deploy).await;
            assert!(
                matches!(result, Err(ApiTokenError::CreatorInactive)),
                "{}: {result:?}",
                db.backend
            );
        }
        let gone = api_token::Entity::find_by_id(id_deleted)
            .one(&env.db)
            .await
            .unwrap();
        assert!(gone.is_none(), "{}: deleted with its creator", db.backend);
    })
    .await;
}

#[tokio::test]
async fn last_used_at_is_written_at_most_once_a_minute() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, raw) = env.issue(None, &[Scope::Deploy]).await;
        let at = |ms: i64| Some(time::from_millis(ms).unwrap());

        assert_eq!(env.row(id).await.last_used_at, None, "{}", db.backend);
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        assert_eq!(env.row(id).await.last_used_at, at(T0_MS), "{}", db.backend);
        env.clock.advance(Duration::from_secs(30));
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        assert_eq!(env.row(id).await.last_used_at, at(T0_MS), "{}", db.backend);
        env.clock.advance(Duration::from_secs(30));
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        let now = env.clock.now_ms();
        assert_eq!(env.row(id).await.last_used_at, at(now), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn list_shows_every_token_without_its_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (a, _) = env.issue(None, &[Scope::Deploy]).await;
        let (b, raw_b) = env
            .issue(None, &[Scope::AdminRead, Scope::AdminWrite])
            .await;
        env.tokens.revoke(a).await.unwrap();

        let listed = env.tokens.list().await.unwrap();

        let ids: Vec<i64> = listed.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a, b], "{}", db.backend);
        assert!(listed[0].revoked_at.is_some(), "{}", db.backend);
        assert_eq!(
            listed[1].scopes,
            BTreeSet::from([Scope::AdminRead, Scope::AdminWrite]),
            "{}",
            db.backend
        );
        assert!(raw_b.starts_with(&listed[1].token_prefix), "{}", db.backend);
    })
    .await;
}

#[test]
fn scopes_have_stable_wire_names() {
    let names: Vec<&str> = [
        Scope::Deploy,
        Scope::Assign,
        Scope::AdminRead,
        Scope::AdminWrite,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect();

    assert_eq!(names, ["deploy", "assign", "admin:read", "admin:write"]);
    for name in names {
        assert_eq!(Scope::parse(name).map(Scope::as_str), Some(name));
    }
    assert_eq!(Scope::parse("admin"), None);
}
