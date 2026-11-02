//! The core half of the console's account setup (M3-02): the session principal's account
//! flags, the self-service password change (verified, audited in its transaction, a new
//! stamp for the fresh session) and the rotation of a console session after a privilege
//! change. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::{Clock, ManualClock, SystemClock};
use deskhaven_core::console_audit::{Actor, NewConsoleAudit};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::sessions::{
    AccountFlags, ClientDevice, SessionError, SessionKind, SessionService, SessionToken,
};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    AuthSource, Credential, CredentialStamp, HashParams, NewUser, Password, PasswordHasher,
    ProfileUpdate, UserError, UserService,
};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

const PASSWORD: &str = "tundra-Lamp-42";
const NEW_PASSWORD: &str = "glacier-Pen-1984";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

struct Env {
    users: UserService,
    sessions: SessionService,
}

impl Env {
    async fn new(db: &TestDb) -> Self {
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
        let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
            .await
            .unwrap();
        let clock = Arc::new(ManualClock::at(SystemClock.now_ms()));
        let users = UserService::with_clock(
            db.conn.clone(),
            settings.clone(),
            PasswordHasher::new(FAST).unwrap(),
            clock.clone(),
        );
        let sessions = SessionService::new(db.conn.clone(), settings, users.clone(), clock);
        Self { users, sessions }
    }

    async fn user(&self, username: &str) -> (i64, CredentialStamp) {
        let password = Password::new(PASSWORD);
        let id = self
            .users
            .create(NewUser::new(username, Credential::Local { password }))
            .await
            .unwrap()
            .id;
        let stamp = self
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap()
            .stamp;
        (id, stamp)
    }

    async fn console(&self, user_id: i64, stamp: &CredentialStamp) -> (i64, SessionToken) {
        let (session, token) = self
            .sessions
            .create_console(user_id, stamp, None, Some("agent"))
            .await
            .unwrap();
        (session.id, token)
    }

    async fn client(&self, user_id: i64, stamp: &CredentialStamp) -> SessionToken {
        let device = ClientDevice {
            rustdesk_id: "123456789".to_owned(),
            uuid: "uuid".to_owned(),
            info: None,
        };
        self.sessions
            .create_client(user_id, stamp, &device, None)
            .await
            .unwrap()
            .1
    }

    async fn principal(
        &self,
        token: &SessionToken,
        kind: SessionKind,
    ) -> Result<i64, SessionError> {
        self.sessions
            .authenticate(token.expose(), kind)
            .await
            .map(|p| p.session_id)
    }
}

fn audit(user_id: i64) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: Actor::User(user_id),
        action: "user.password_change".to_owned(),
        target_type: "user".to_owned(),
        target_id: user_id.to_string(),
        before: None,
        after: None,
        ip: None,
    }
}

async fn audit_rows(db: &TestDb, action: &str) -> usize {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn the_principal_carries_the_account_flags() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, stamp) = env.user("alice").await;
        let update = ProfileUpdate {
            must_change_password: Some(true),
            ..ProfileUpdate::default()
        };
        env.users.update_profile(id, update).await.unwrap();
        let (_, token) = env.console(id, &stamp).await;

        let principal = env
            .sessions
            .authenticate(token.expose(), SessionKind::Console)
            .await
            .unwrap();

        assert_eq!(
            principal.account,
            AccountFlags {
                auth_source: AuthSource::Local,
                must_change_password: true,
                totp_enabled: false,
            },
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_password_change_verifies_revokes_audits_and_returns_the_new_stamp() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, stamp) = env.user("alice").await;
        env.users
            .update_profile(
                id,
                ProfileUpdate {
                    must_change_password: Some(true),
                    ..ProfileUpdate::default()
                },
            )
            .await
            .unwrap();
        let (_, old) = env.console(id, &stamp).await;
        let b = db.backend;

        let new_stamp = env
            .users
            .change_password(
                id,
                &Password::new(PASSWORD),
                &Password::new(NEW_PASSWORD),
                &audit(id),
            )
            .await
            .unwrap();

        assert!(
            env.principal(&old, SessionKind::Console).await.is_err(),
            "{b}: old session revoked"
        );
        let fresh = env
            .sessions
            .create_console(id, &new_stamp, None, None)
            .await;
        assert!(fresh.is_ok(), "{b}: the returned stamp is current");
        assert!(
            env.sessions
                .create_console(id, &stamp, None, None)
                .await
                .is_err(),
            "{b}: the old stamp is stale"
        );
        let login = env
            .users
            .authenticate_local("alice", &Password::new(NEW_PASSWORD))
            .await
            .unwrap();
        assert!(!login.user.must_change_password, "{b}");
        assert_eq!(login.stamp, new_stamp, "{b}");
        assert_eq!(audit_rows(db, "user.password_change").await, 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_wrong_current_password_or_a_weak_new_one_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, stamp) = env.user("alice").await;
        let (_, session) = env.console(id, &stamp).await;
        let b = db.backend;

        let wrong = env
            .users
            .change_password(
                id,
                &Password::new("not-the-password"),
                &Password::new(NEW_PASSWORD),
                &audit(id),
            )
            .await;
        let weak = env
            .users
            .change_password(
                id,
                &Password::new(PASSWORD),
                &Password::new("short"),
                &audit(id),
            )
            .await;

        assert!(
            matches!(wrong, Err(UserError::WrongPassword)),
            "{b}: {wrong:?}"
        );
        assert!(
            matches!(weak, Err(UserError::PolicyViolation { .. })),
            "{b}: {weak:?}"
        );
        assert!(
            env.principal(&session, SessionKind::Console).await.is_ok(),
            "{b}"
        );
        assert_eq!(audit_rows(db, "user.password_change").await, 0, "{b}");
    })
    .await;
}

#[tokio::test]
async fn rotation_replaces_every_console_session_and_keeps_client_sessions() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, stamp) = env.user("alice").await;
        let (current, current_token) = env.console(id, &stamp).await;
        let (_, other_console) = env.console(id, &stamp).await;
        let client = env.client(id, &stamp).await;
        let (_, bob) = {
            let (bob_id, bob_stamp) = env.user("bob").await;
            env.console(bob_id, &bob_stamp).await
        };
        let b = db.backend;

        let (rotated, token) = env
            .sessions
            .rotate_console(current, None, Some("agent"))
            .await
            .unwrap();

        assert_ne!(rotated.id, current, "{b}");
        assert_eq!(rotated.kind, SessionKind::Console, "{b}");
        assert_eq!(
            env.principal(&token, SessionKind::Console).await.unwrap(),
            rotated.id,
            "{b}"
        );
        for revoked in [&current_token, &other_console] {
            assert!(
                env.principal(revoked, SessionKind::Console).await.is_err(),
                "{b}"
            );
        }
        assert!(
            env.principal(&client, SessionKind::Client).await.is_ok(),
            "{b}"
        );
        assert!(
            env.principal(&bob, SessionKind::Console).await.is_ok(),
            "{b}"
        );
        let again = env.sessions.rotate_console(current, None, None).await;
        assert!(
            matches!(again, Err(SessionError::Revoked)),
            "{b}: {again:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn rotation_refuses_unknown_and_client_sessions() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, stamp) = env.user("alice").await;
        let client = env.client(id, &stamp).await;
        let client_id = env
            .sessions
            .authenticate(client.expose(), SessionKind::Client)
            .await
            .unwrap()
            .session_id;
        let b = db.backend;

        let unknown = env.sessions.rotate_console(i64::MAX, None, None).await;
        let of_client = env.sessions.rotate_console(client_id, None, None).await;

        assert!(matches!(unknown, Err(SessionError::InvalidToken)), "{b}");
        assert!(matches!(of_client, Err(SessionError::InvalidToken)), "{b}");
        assert!(
            env.principal(&client, SessionKind::Client).await.is_ok(),
            "{b}"
        );
    })
    .await;
}
