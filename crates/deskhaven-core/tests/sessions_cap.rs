//! Sessions per user: creating one beyond `auth.max_sessions_per_user` revokes the least
//! recently used of the user's usable sessions (cache included), and the user's session
//! list is paged. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::{MINUTE, clock, set, settings};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::sessions::{ClientDevice, SessionError, SessionKind, SessionService};
use deskhaven_core::sessions::{Session, SessionToken};
use deskhaven_core::users::{
    Credential, CredentialStamp, HashParams, NewUser, Password, PasswordHasher, UserService,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;
use std::sync::Arc;

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

struct Env {
    users: UserService,
    sessions: SessionService,
    clock: Arc<ManualClock>,
}

impl Env {
    /// Services with `auth.max_sessions_per_user` = `cap`.
    async fn new(db: &TestDb, cap: u32) -> Self {
        let settings = settings(db).await;
        set(&settings, "auth.max_sessions_per_user", json!(cap)).await;
        let clock = clock();
        let hasher = PasswordHasher::new(FAST).unwrap();
        let users =
            UserService::with_clock(db.conn.clone(), settings.clone(), hasher, clock.clone());
        let sessions = SessionService::new(db.conn.clone(), settings, users.clone(), clock.clone());
        Self {
            users,
            sessions,
            clock,
        }
    }

    async fn login(&self, username: &str) -> (i64, CredentialStamp) {
        let password = Password::new(PASSWORD);
        self.users
            .create(NewUser::new(username, Credential::Local { password }))
            .await
            .unwrap();
        let login = self
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap();
        (login.user.id, login.stamp)
    }

    /// A console session of `user_id`, one minute after the previous one.
    async fn console(&self, user_id: i64, stamp: &CredentialStamp) -> (Session, SessionToken) {
        self.clock.advance(MINUTE);
        self.sessions
            .create_console(user_id, stamp, None, None)
            .await
            .unwrap()
    }

    async fn auth(&self, token: &SessionToken, kind: SessionKind) -> Result<i64, SessionError> {
        let principal = self.sessions.authenticate(token.expose(), kind).await;
        principal.map(|p| p.user_id)
    }

    async fn listed(&self, user_id: i64) -> Vec<i64> {
        let page = PageRequest {
            page: 1,
            page_size: 200,
        };
        let found = self.sessions.list_for_user(user_id, page).await.unwrap();
        found.items.iter().map(|s| s.id).collect()
    }
}

fn device(rustdesk_id: &str) -> ClientDevice {
    ClientDevice {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: "dXVpZA==".to_owned(),
        info: None,
    }
}

#[tokio::test]
async fn a_session_beyond_the_cap_revokes_the_least_recently_used_one() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db, 3).await;
        let (alice, stamp) = env.login("alice").await;
        let (first, first_token) = env.console(alice, &stamp).await;
        // Cached (and not touched: just created) before the eviction, which must reach the
        // cache too.
        let b = db.backend;
        assert_eq!(
            env.auth(&first_token, SessionKind::Console).await.ok(),
            Some(alice),
            "{b}"
        );
        let (second, _) = env.console(alice, &stamp).await;
        env.clock.advance(MINUTE);
        let (third, _) = env
            .sessions
            .create_client(alice, &stamp, &device("111111111"), None)
            .await
            .unwrap();

        let (fourth, _) = env.console(alice, &stamp).await;

        let revoked = env.auth(&first_token, SessionKind::Console).await;
        assert!(
            matches!(revoked, Err(SessionError::Revoked)),
            "{b}: {revoked:?}"
        );
        assert_eq!(
            env.listed(alice).await,
            [fourth.id, third.id, second.id],
            "{b}"
        );
        assert_ne!(first.id, fourth.id);
    })
    .await;
}

#[tokio::test]
async fn the_cap_counts_only_the_users_own_usable_sessions() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db, 2).await;
        let (alice, alice_stamp) = env.login("alice").await;
        let (bob, bob_stamp) = env.login("bob").await;
        let (revoked, _) = env.console(alice, &alice_stamp).await;
        env.sessions.revoke(revoked.id).await.unwrap();
        let (bobs, _) = env.console(bob, &bob_stamp).await;
        let (kept, _) = env.console(alice, &alice_stamp).await;

        let (newest, _) = env.console(alice, &alice_stamp).await;

        let b = db.backend;
        assert_eq!(env.listed(alice).await, [newest.id, kept.id], "{b}");
        assert_eq!(env.listed(bob).await, [bobs.id], "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_session_list_is_paged_most_recent_first() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db, 10).await;
        let (alice, stamp) = env.login("alice").await;
        let mut ids = Vec::new();
        for _ in 0..3 {
            ids.push(env.console(alice, &stamp).await.0.id);
        }
        let page = |page| PageRequest { page, page_size: 2 };

        let first = env.sessions.list_for_user(alice, page(1)).await.unwrap();
        let second = env.sessions.list_for_user(alice, page(2)).await.unwrap();
        let invalid = env.sessions.list_for_user(alice, page(0)).await;

        let b = db.backend;
        let of = |p: &deskhaven_core::pagination::Page<Session>| {
            p.items.iter().map(|s| s.id).collect::<Vec<_>>()
        };
        assert_eq!(of(&first), [ids[2], ids[1]], "{b}");
        assert_eq!(of(&second), [ids[0]], "{b}");
        assert_eq!((first.total, first.page, first.page_size), (3, 1, 2), "{b}");
        assert!(
            matches!(invalid, Err(SessionError::Invalid { field: "page", .. })),
            "{b}"
        );
    })
    .await;
}

/// Using a session makes it recent: the cap ends the least recently *used* one.
#[tokio::test]
async fn a_session_used_lately_survives_the_cap() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db, 2).await;
        let (alice, stamp) = env.login("alice").await;
        let (_, first_token) = env.console(alice, &stamp).await;
        let (second, _) = env.console(alice, &stamp).await;
        env.clock.advance(MINUTE);
        // Past the touch interval: `last_used_at` moves forward.
        env.auth(&first_token, SessionKind::Console).await.unwrap();

        let (third, _) = env.console(alice, &stamp).await;

        let b = db.backend;
        let still = env.auth(&first_token, SessionKind::Console).await;
        assert_eq!(still.ok(), Some(alice), "{b}");
        let listed = env.listed(alice).await;
        assert!(
            !listed.contains(&second.id) && listed.contains(&third.id),
            "{b}"
        );
    })
    .await;
}
