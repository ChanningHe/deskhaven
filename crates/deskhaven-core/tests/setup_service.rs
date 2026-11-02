//! First-run setup: token issue at startup, single use, expiry, owner-exists refusal,
//! retirement of older tokens and the audit row written with the owner. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use deskhaven_core::clock::ManualClock;
use deskhaven_core::console_audit::{Actor, NewConsoleAudit};
use deskhaven_core::setup::{
    NewOwner, SETUP_TOKEN_TTL, SetupError, SetupService, SetupToken, StartupSetup,
};
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, Role, User, UserError, UserService,
};
use deskhaven_db::entity::{console_audit, setup_token};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

struct Env {
    setup: SetupService,
    users: UserService,
    clock: Arc<ManualClock>,
}

impl Env {
    async fn new(db: &TestDb) -> Self {
        let settings = common::settings(db).await;
        let clock = common::clock();
        let users = UserService::with_clock(
            db.conn.clone(),
            settings,
            PasswordHasher::new(FAST).unwrap(),
            clock.clone(),
        );
        let setup = SetupService::new(db.conn.clone(), users.clone(), clock.clone());
        Self {
            setup,
            users,
            clock,
        }
    }

    async fn complete(&self, token: &str, username: &str) -> Result<User, SetupError> {
        let owner = NewOwner {
            username: username.to_owned(),
            display_name: String::new(),
            email: None,
            password: Password::new(PASSWORD),
        };
        self.setup
            .complete(token, owner, |user| audit(user.id))
            .await
            .map(|done| done.user)
    }

    async fn issued(&self) -> SetupToken {
        match self.setup.on_startup().await.unwrap() {
            StartupSetup::Issued(token) => token,
            other => panic!("expected a new token, got {other:?}"),
        }
    }
}

fn audit(user_id: i64) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: Actor::User(user_id),
        action: "setup.complete".to_owned(),
        target_type: "user".to_owned(),
        target_id: user_id.to_string(),
        before: None,
        after: None,
        ip: None,
    }
}

#[tokio::test]
async fn startup_issues_once_then_reports_the_pending_token() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let b = db.backend;

        assert!(env.setup.is_required().await.unwrap(), "{b}");
        let token = env.issued().await;
        let second = env.setup.on_startup().await.unwrap();

        assert_eq!(token.expose().len(), 43, "{b}");
        assert!(
            matches!(second, StartupSetup::Pending { expires_at } if expires_at == token.expires_at),
            "{b}: {second:?}"
        );
        let stored = setup_token::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(stored.len(), 1, "{b}");
        assert_ne!(stored[0].token_hash, token.expose(), "{b}: only the hash is stored");
        // Expired: the next start issues a fresh one.
        env.clock.advance(SETUP_TOKEN_TTL);
        assert!(
            matches!(env.setup.on_startup().await.unwrap(), StartupSetup::Issued(_)),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_token_creates_the_owner_once_with_its_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let token = env.issued().await;
        let b = db.backend;

        let owner = env.complete(token.expose(), "root").await.unwrap();
        let reused = env.complete(token.expose(), "root2").await;

        assert_eq!(owner.role, Role::Owner, "{b}");
        assert!(!owner.must_change_password, "{b}");
        assert!(
            matches!(reused, Err(SetupError::InvalidToken)),
            "{b}: {reused:?}"
        );
        assert!(!env.setup.is_required().await.unwrap(), "{b}");
        assert!(
            matches!(env.setup.on_startup().await.unwrap(), StartupSetup::Done),
            "{b}"
        );
        let rows = console_audit::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].action, "setup.complete", "{b}");
        assert_eq!(rows[0].actor_id, Some(owner.id), "{b}");
        let login = env
            .users
            .authenticate_local("root", &Password::new(PASSWORD))
            .await;
        assert!(login.is_ok(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn bad_expired_and_retired_tokens_are_refused_without_side_effects() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let first = env.issued().await;
        let second = env.setup.issue().await.unwrap();
        let b = db.backend;

        let retired = env.complete(first.expose(), "root").await;
        let malformed = env.complete("not-a-token", "root").await;
        let mut tampered = second.expose().to_owned();
        tampered.replace_range(..1, if tampered.starts_with('A') { "B" } else { "A" });
        let unknown = env.complete(&tampered, "root").await;
        env.clock.advance(SETUP_TOKEN_TTL);
        let expired = env.complete(second.expose(), "root").await;

        for (name, result) in [
            ("retired", retired),
            ("malformed", malformed),
            ("unknown", unknown),
            ("expired", expired),
        ] {
            assert!(
                matches!(result, Err(SetupError::InvalidToken)),
                "{b}: {name}: {result:?}"
            );
        }
        assert!(env.setup.is_required().await.unwrap(), "{b}");
        let audits = console_audit::Entity::find().count(&db.conn).await.unwrap();
        assert_eq!(audits, 0, "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_existing_owner_refuses_issue_and_completion() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let token = env.issued().await;
        let new = NewUser {
            role: Role::Owner,
            ..NewUser::new(
                "existing",
                Credential::Local {
                    password: Password::new(PASSWORD),
                },
            )
        };
        env.users.create(new).await.unwrap();
        let b = db.backend;

        let completed = env.complete(token.expose(), "root").await;
        let issued = env.setup.issue().await;

        assert!(
            matches!(completed, Err(SetupError::OwnerExists)),
            "{b}: {completed:?}"
        );
        assert!(
            matches!(issued, Err(SetupError::OwnerExists)),
            "{b}: {issued:?}"
        );
        assert!(env.users.get_by_username("root").await.is_err(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_refused_account_leaves_the_token_usable() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let token = env.issued().await;
        let b = db.backend;

        let weak = NewOwner {
            username: "root".to_owned(),
            display_name: String::new(),
            email: None,
            password: Password::new("short"),
        };
        let refused = env
            .setup
            .complete(token.expose(), weak, |u| audit(u.id))
            .await;
        let bad_name = env.complete(token.expose(), "bad name!").await;
        let ok = env.complete(token.expose(), "root").await;

        assert!(
            matches!(
                refused,
                Err(SetupError::User(UserError::PolicyViolation { .. }))
            ),
            "{b}: {refused:?}"
        );
        assert!(
            matches!(bad_name, Err(SetupError::User(UserError::Invalid { .. }))),
            "{b}: {bad_name:?}"
        );
        assert!(ok.is_ok(), "{b}: {ok:?}");
    })
    .await;
}
