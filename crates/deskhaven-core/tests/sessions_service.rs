//! SessionService: hashed tokens, creation under the credential stamp, absolute and idle
//! expiry with a manual clock, throttled touches, the cache (hits, invalidation, flush),
//! read-time owner checks, user events and cleanup. Every test runs on SQLite and
//! PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::{Clock, ManualClock, SystemClock};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::sessions::{
    CachedAuth, ClientDevice, SessionError, SessionKind, SessionService, SessionToken,
};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, CredentialStamp, HashParams, NewUser, Password, PasswordHasher, Role, Status,
    UserChange, UserEvent, UserService,
};
use deskhaven_db::entity::{session, user};
use deskhaven_db::testing::{Backend, TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;

const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};
const MINUTE: Duration = Duration::from_secs(60);
const DAY: Duration = Duration::from_secs(24 * 3600);
/// Defaults of `auth.session_ttl` / `auth.session_idle_ttl`.
const SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const IDLE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

struct Env {
    users: UserService,
    sessions: SessionService,
    clock: Arc<ManualClock>,
    /// Statements run on the sessions service's connection.
    queries: Arc<AtomicUsize>,
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
        let queries = Arc::new(AtomicUsize::new(0));
        let mut counted = db.conn.clone();
        let counter = Arc::clone(&queries);
        counted.set_metric_callback(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        let sessions = SessionService::new(counted, settings, users.clone(), clock.clone());
        Self {
            users,
            sessions,
            clock,
            queries,
        }
    }

    /// A local account and the stamp of a successful login.
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

    async fn client_session(&self, username: &str) -> (i64, SessionToken) {
        let (user_id, stamp) = self.login(username).await;
        let (_, token) = self
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await
            .unwrap();
        (user_id, token)
    }

    fn queries(&self) -> usize {
        self.queries.load(Ordering::SeqCst)
    }

    /// Authenticates a client token; the owner's id.
    async fn auth(&self, token: &SessionToken) -> Result<i64, SessionError> {
        self.auth_as(token, SessionKind::Client).await
    }

    async fn auth_console(&self, token: &SessionToken) -> Result<i64, SessionError> {
        self.auth_as(token, SessionKind::Console).await
    }

    async fn auth_as(&self, token: &SessionToken, kind: SessionKind) -> Result<i64, SessionError> {
        self.sessions
            .authenticate(token.expose(), kind)
            .await
            .map(|p| p.user_id)
    }

    /// Records, from a commit hook registered after the session service's own, how many
    /// sessions are still cached at each committed change: hooks run before the event is
    /// broadcast, so the event consumer cannot have touched the cache yet.
    fn observe_cache_at_commit(&self) -> Arc<Mutex<Vec<usize>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (cache, sink) = (self.sessions.cache().clone(), Arc::clone(&seen));
        self.users
            .register_commit_hook(Arc::new(move |_: &UserChange| {
                sink.lock().unwrap().push(cache.len());
            }));
        seen
    }
}

fn device() -> ClientDevice {
    ClientDevice {
        rustdesk_id: "123456789".to_owned(),
        uuid: "6b0c3c7e-device".to_owned(),
        info: Some(serde_json::json!({ "os": "linux", "type": "client", "name": "host" })),
    }
}

async fn row(db: &TestDb, session_id: i64) -> Option<session::Model> {
    session::Entity::find_by_id(session_id)
        .one(&db.conn)
        .await
        .unwrap()
}

async fn session_count(db: &TestDb) -> usize {
    session::Entity::find().all(&db.conn).await.unwrap().len()
}

#[tokio::test]
async fn only_the_sha256_of_the_token_is_stored() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let ip: IpAddr = "198.51.100.4".parse().unwrap();

        let (session, token) = env
            .sessions
            .create_client(user_id, &stamp, &device(), Some(ip))
            .await
            .unwrap();

        let stored = row(db, session.id).await.unwrap();
        let expected = format!("{:x}", Sha256::digest(token.expose().as_bytes()));
        assert_eq!(stored.token_hash, expected);
        let dump = format!("{stored:?}");
        assert!(
            !dump.contains(token.expose()),
            "raw token in the row: {dump}"
        );
        assert!(!format!("{session:?} {token:?}").contains(token.expose()));
        assert_eq!(session.ip, Some(ip));
        assert_eq!(session.device_info.unwrap()["os"], "linux");
    })
    .await;
}

#[tokio::test]
async fn created_sessions_authenticate_as_their_owner_and_device() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let (client, client_token) = env
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await
            .unwrap();
        let (console, console_token) = env
            .sessions
            .create_console(user_id, &stamp, None, Some("Mozilla/5.0"))
            .await
            .unwrap();

        let as_client = env
            .sessions
            .authenticate(client_token.expose(), SessionKind::Client)
            .await
            .unwrap();
        let as_console = env
            .sessions
            .authenticate(console_token.expose(), SessionKind::Console)
            .await
            .unwrap();

        assert_eq!(as_client.session_id, client.id);
        assert_eq!(as_client.user_id, user_id);
        assert_eq!(as_client.role, Role::User);
        assert_eq!(as_client.kind, SessionKind::Client);
        let bound = as_client.device.unwrap();
        assert_eq!(
            (bound.rustdesk_id.as_str(), bound.uuid.as_str()),
            ("123456789", "6b0c3c7e-device")
        );
        assert_eq!(as_console.session_id, console.id);
        assert_eq!(as_console.kind, SessionKind::Console);
        assert_eq!(as_console.device, None);
        assert_eq!(console.device_info.unwrap()["user_agent"], "Mozilla/5.0");
        let expires = client.expires_at.timestamp_millis() - client.created_at.timestamp_millis();
        assert_eq!(expires, i64::try_from(SESSION_TTL.as_millis()).unwrap());
    })
    .await;
}

#[tokio::test]
async fn unknown_and_malformed_tokens_are_rejected_without_a_query_for_malformed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let unknown = "A".repeat(43);

        let before = env.queries();
        let malformed = env
            .sessions
            .authenticate("not a token", SessionKind::Client)
            .await;
        let after_malformed = env.queries();
        let missing = env
            .sessions
            .authenticate(&unknown, SessionKind::Client)
            .await;

        assert!(matches!(malformed, Err(SessionError::InvalidToken)));
        assert_eq!(after_malformed, before);
        assert!(matches!(missing, Err(SessionError::InvalidToken)));
        assert!(missing.unwrap_err().is_unauthenticated());
    })
    .await;
}

#[tokio::test]
async fn client_sessions_require_a_device_id_and_uuid() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let no_id = ClientDevice {
            rustdesk_id: " ".to_owned(),
            ..device()
        };
        let no_uuid = ClientDevice {
            uuid: String::new(),
            ..device()
        };

        let a = env
            .sessions
            .create_client(user_id, &stamp, &no_id, None)
            .await;
        let b = env
            .sessions
            .create_client(user_id, &stamp, &no_uuid, None)
            .await;

        assert!(matches!(
            a,
            Err(SessionError::InvalidDevice {
                field: "rustdesk_id",
                ..
            })
        ));
        assert!(matches!(
            b,
            Err(SessionError::InvalidDevice { field: "uuid", .. })
        ));
        assert_eq!(session_count(db).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_password_change_after_login_makes_session_creation_fail() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        env.users
            .set_password(user_id, &Password::new("another-Phrase-7"), false)
            .await
            .unwrap();

        let client = env
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await;
        let console = env
            .sessions
            .create_console(user_id, &stamp, None, None)
            .await;

        assert!(matches!(client, Err(SessionError::StaleCredential)));
        assert!(matches!(console, Err(SessionError::StaleCredential)));
        assert_eq!(session_count(db).await, 0);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_created_during_a_password_change_never_stays_valid() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        for round in 0..8 {
            let (user_id, stamp) = env.login(&format!("racer{round}")).await;
            let users = env.users.clone();
            let change = tokio::spawn(async move {
                users
                    .set_password(user_id, &Password::new("another-Phrase-7"), false)
                    .await
            });
            let sessions = env.sessions.clone();
            let create = tokio::spawn(async move {
                sessions
                    .create_client(user_id, &stamp, &device(), None)
                    .await
            });
            change.await.unwrap().unwrap();

            // Either creation lost the race, or the change revoked what it created.
            match create.await.unwrap() {
                Err(SessionError::StaleCredential) => {}
                Ok((session, _)) => {
                    let stored = row(db, session.id).await.unwrap();
                    assert!(
                        stored.revoked_at.is_some(),
                        "round {round}: session survived"
                    );
                }
                Err(other) => panic!("round {round}: {other:?}"),
            }
        }
    })
    .await;
}

#[tokio::test]
async fn disabling_or_deleting_after_login_makes_session_creation_fail() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (disabled, disabled_stamp) = env.login("alice").await;
        let (deleted, deleted_stamp) = env.login("bob").await;
        env.users
            .set_status(disabled, Status::Disabled)
            .await
            .unwrap();
        env.users.delete(deleted).await.unwrap();

        let a = env
            .sessions
            .create_client(disabled, &disabled_stamp, &device(), None)
            .await;
        let b = env
            .sessions
            .create_client(deleted, &deleted_stamp, &device(), None)
            .await;

        assert!(matches!(a, Err(SessionError::StaleCredential)));
        assert!(matches!(b, Err(SessionError::StaleCredential)));
    })
    .await;
}

#[tokio::test]
async fn logout_revokes_at_once_even_when_cached() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();
        assert_eq!(env.sessions.cache().len(), 1);

        let revoked = env.sessions.revoke_token(token.expose()).await.unwrap();
        let again = env.sessions.revoke_token(token.expose()).await.unwrap();
        let malformed = env.sessions.revoke_token("junk").await.unwrap();

        assert!(revoked);
        assert!(!again);
        assert!(!malformed);
        assert!(matches!(env.auth(&token).await, Err(SessionError::Revoked)));
    })
    .await;
}

#[tokio::test]
async fn revoke_by_id_and_for_a_user_leave_other_users_alone() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, stamp) = env.login("alice").await;
        let (first, first_token) = env
            .sessions
            .create_client(alice, &stamp, &device(), None)
            .await
            .unwrap();
        let (_, second_token) = env
            .sessions
            .create_client(alice, &stamp, &device(), None)
            .await
            .unwrap();
        let (_, third_token) = env
            .sessions
            .create_console(alice, &stamp, None, None)
            .await
            .unwrap();
        let (bob, bob_token) = env.client_session("bob").await;
        for token in [&first_token, &second_token, &bob_token] {
            env.auth(token).await.unwrap();
        }
        env.auth_console(&third_token).await.unwrap();

        assert!(env.sessions.revoke(first.id).await.unwrap());
        assert!(!env.sessions.revoke(first.id).await.unwrap());
        assert!(matches!(
            env.auth(&first_token).await,
            Err(SessionError::Revoked)
        ));
        assert_eq!(env.auth(&second_token).await.unwrap(), alice);

        assert_eq!(env.sessions.revoke_all_for_user(alice).await.unwrap(), 2);
        assert!(matches!(
            env.auth(&second_token).await,
            Err(SessionError::Revoked)
        ));
        assert!(matches!(
            env.auth_console(&third_token).await,
            Err(SessionError::Revoked)
        ));
        assert_eq!(env.auth(&bob_token).await.unwrap(), bob);
    })
    .await;
}

#[tokio::test]
async fn absolute_expiry_applies_even_to_a_session_in_constant_use() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;

        // Used every 6 days: never idle, but 30 days old at the end.
        let mut elapsed = Duration::ZERO;
        while elapsed + 6 * DAY < SESSION_TTL {
            env.clock.advance(6 * DAY);
            elapsed += 6 * DAY;
            env.auth(&token).await.unwrap();
        }
        env.clock
            .advance(SESSION_TTL - elapsed - Duration::from_millis(1));
        env.auth(&token).await.unwrap();
        env.clock.advance(Duration::from_millis(1));

        assert!(matches!(env.auth(&token).await, Err(SessionError::Expired)));
    })
    .await;
}

#[tokio::test]
async fn idle_expiry_slides_with_use() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, used) = env.client_session("alice").await;
        let (_, unused) = env.client_session("bob").await;

        env.clock.advance(IDLE_TTL - MINUTE);
        env.auth(&used).await.unwrap();
        env.clock.advance(IDLE_TTL - MINUTE);
        let still_used = env.auth(&used).await;
        let unused_result = env.auth(&unused).await;
        env.clock.advance(IDLE_TTL);
        let after_idle = env.auth(&used).await;

        assert!(still_used.is_ok());
        assert!(matches!(unused_result, Err(SessionError::Expired)));
        assert!(matches!(after_idle, Err(SessionError::Expired)));
    })
    .await;
}

#[tokio::test]
async fn expiry_is_enforced_on_a_cache_hit() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let (session, token) = env
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await
            .unwrap();
        // Expires 30s from now; cached now, then crossed within the cache TTL.
        let soon = env.clock.now_ms() + 30_000;
        session::Entity::update_many()
            .col_expr(session::Column::ExpiresAt, Expr::value(soon))
            .filter(session::Column::Id.eq(session.id))
            .exec(&db.conn)
            .await
            .unwrap();
        env.auth(&token).await.unwrap();
        env.clock.advance(Duration::from_secs(30));
        let before = env.queries();

        let result = env.auth(&token).await;

        assert!(matches!(result, Err(SessionError::Expired)));
        assert_eq!(env.queries(), before, "decided from the cache");
        assert!(env.sessions.cache().is_empty());
    })
    .await;
}

#[tokio::test]
async fn cache_hits_and_fresh_touches_cost_no_query() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;

        let start = env.queries();
        env.auth(&token).await.unwrap();
        let first = env.queries() - start;
        for _ in 0..10 {
            env.clock.advance(Duration::from_secs(5));
            env.auth(&token).await.unwrap();
        }
        let repeated = env.queries() - start - first;

        assert_eq!(
            first, 1,
            "one SELECT on the miss, no touch for a fresh session"
        );
        assert_eq!(repeated, 0, "cache hits within the touch interval");
    })
    .await;
}

#[tokio::test]
async fn last_used_at_is_written_at_most_once_per_interval() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let (session, token) = env
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await
            .unwrap();
        let created = session.last_used_at.timestamp_millis();

        // Cache filled at +59s (no touch yet: last use is 59s old).
        env.clock.advance(Duration::from_secs(59));
        env.auth(&token).await.unwrap();
        let unchanged = row(db, session.id)
            .await
            .unwrap()
            .last_used_at
            .timestamp_millis();
        // +60s: a cache hit, but the last use is now a full interval old.
        env.clock.advance(Duration::from_secs(1));
        let before_touch = env.queries();
        env.auth(&token).await.unwrap();
        let touch_queries = env.queries() - before_touch;
        let touched = row(db, session.id)
            .await
            .unwrap()
            .last_used_at
            .timestamp_millis();
        // +61s .. +119s: nothing more to write.
        let before_quiet = env.queries();
        for _ in 0..58 {
            env.clock.advance(Duration::from_secs(1));
            env.auth(&token).await.unwrap();
        }
        let quiet_queries = env.queries() - before_quiet;

        assert_eq!(unchanged, created);
        assert_eq!(touch_queries, 1, "one UPDATE, no SELECT");
        assert_eq!(touched, created + 60_000);
        assert_eq!(quiet_queries, 0);
    })
    .await;
}

#[tokio::test]
async fn a_revocation_racing_a_cache_fill_never_leaves_a_stale_entry() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;

        // Many authentications race one logout; afterwards nothing may accept the token.
        let racers: Vec<_> = (0..16)
            .map(|_| {
                let sessions = env.sessions.clone();
                let token = token.expose().to_owned();
                tokio::spawn(
                    async move { sessions.authenticate(&token, SessionKind::Client).await },
                )
            })
            .collect();
        env.sessions.revoke_token(token.expose()).await.unwrap();
        for racer in racers {
            let _ = racer.await.unwrap();
        }

        assert!(matches!(env.auth(&token).await, Err(SessionError::Revoked)));
    })
    .await;
}

#[tokio::test]
async fn a_user_disabled_behind_the_services_back_is_rejected_at_read_time() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, cached) = env.client_session("alice").await;
        env.auth(&cached).await.unwrap();
        let (_, uncached) = env.client_session("bob").await;
        let bob = env.users.get_by_username("bob").await.unwrap().id;

        // A direct write: no event, no in-transaction revocation (a crash, another node).
        for id in [alice, bob] {
            let change = user::ActiveModel {
                id: Set(id),
                status: Set(Status::Disabled),
                ..Default::default()
            };
            change.update(&db.conn).await.unwrap();
        }
        let fresh = env.auth(&uncached).await;
        let within_ttl = env.auth(&cached).await;
        env.clock.advance(MINUTE);
        let after_ttl = env.auth(&cached).await;

        assert!(matches!(fresh, Err(SessionError::UserInactive)));
        assert!(within_ttl.is_ok(), "the cache TTL bounds the staleness");
        assert!(matches!(after_ttl, Err(SessionError::UserInactive)));
    })
    .await;
}

#[tokio::test]
async fn a_user_deleted_behind_the_services_back_is_rejected_after_the_cache_ttl() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();

        user::Entity::delete_by_id(alice)
            .exec(&db.conn)
            .await
            .unwrap();
        env.clock.advance(MINUTE);

        assert!(matches!(
            env.auth(&token).await,
            Err(SessionError::InvalidToken)
        ));
    })
    .await;
}

async fn unrevoked_sessions_of(db: &TestDb, user_id: i64) -> usize {
    session::Entity::find()
        .filter(session::Column::UserId.eq(user_id))
        .filter(session::Column::RevokedAt.is_null())
        .all(&db.conn)
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn disabling_rejects_a_cached_token_on_the_very_next_request() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();
        let at_commit = env.observe_cache_at_commit();

        env.users.set_status(alice, Status::Disabled).await.unwrap();
        let next = env.auth(&token).await;
        let unrevoked = unrevoked_sessions_of(db, alice).await;
        env.users.set_status(alice, Status::Active).await.unwrap();

        assert_eq!(at_commit.lock().unwrap()[0], 0, "evicted synchronously");
        assert!(matches!(next, Err(SessionError::Revoked)), "{next:?}");
        assert_eq!(unrevoked, 0, "revoked in the same transaction");
        assert!(
            matches!(env.auth(&token).await, Err(SessionError::Revoked)),
            "re-enabling must not resurrect the session"
        );
    })
    .await;
}

#[tokio::test]
async fn a_password_change_rejects_a_cached_token_on_the_very_next_request() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();
        let at_commit = env.observe_cache_at_commit();

        env.users
            .set_password(alice, &Password::new("another-Phrase-7"), false)
            .await
            .unwrap();
        let next = env.auth(&token).await;

        assert_eq!(*at_commit.lock().unwrap(), [0], "evicted synchronously");
        assert!(matches!(next, Err(SessionError::Revoked)), "{next:?}");
        assert_eq!(unrevoked_sessions_of(db, alice).await, 0);
    })
    .await;
}

#[tokio::test]
async fn deleting_rejects_a_cached_token_on_the_very_next_request() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();
        let at_commit = env.observe_cache_at_commit();

        env.users.delete(alice).await.unwrap();
        let next = env.auth(&token).await;

        assert_eq!(*at_commit.lock().unwrap(), [0], "evicted synchronously");
        assert!(matches!(next, Err(SessionError::InvalidToken)), "{next:?}");
    })
    .await;
}

#[tokio::test]
async fn role_and_status_changes_apply_on_the_very_next_request() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        let (bob, bob_token) = env.client_session("bob").await;
        env.auth(&token).await.unwrap();
        env.auth(&bob_token).await.unwrap();

        env.users.set_role(alice, Role::Admin).await.unwrap();
        let promoted = env
            .sessions
            .authenticate(token.expose(), SessionKind::Client)
            .await
            .unwrap();
        env.users.set_status(bob, Status::Unverified).await.unwrap();
        let unverified = env.auth(&bob_token).await;

        assert_eq!(promoted.role, Role::Admin);
        assert!(
            matches!(unverified, Err(SessionError::UserInactive)),
            "{unverified:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_only_authenticates_as_its_own_kind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, stamp) = env.login("alice").await;
        let (_, client) = env
            .sessions
            .create_client(alice, &stamp, &device(), None)
            .await
            .unwrap();
        let (_, console) = env
            .sessions
            .create_console(alice, &stamp, None, None)
            .await
            .unwrap();

        // Twice each: from the database, then from the cache.
        for _ in 0..2 {
            assert!(matches!(
                env.auth_console(&client).await,
                Err(SessionError::InvalidToken)
            ));
            assert!(matches!(
                env.auth(&console).await,
                Err(SessionError::InvalidToken)
            ));
            assert_eq!(env.auth(&client).await.unwrap(), alice);
            assert_eq!(env.auth_console(&console).await.unwrap(), alice);
        }
    })
    .await;
}

/// Makes every `UPDATE ... SET last_used_at` on `sessions` fail.
async fn break_touches(db: &TestDb) {
    let statements: &[&str] = match db.backend {
        Backend::Sqlite => &[
            "CREATE TRIGGER fail_touch BEFORE UPDATE OF last_used_at ON sessions \
             BEGIN SELECT RAISE(ABORT, 'induced touch failure'); END",
        ],
        Backend::Postgres => &[
            "CREATE FUNCTION fail_touch() RETURNS trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'induced touch failure'; END $$",
            "CREATE TRIGGER fail_touch BEFORE UPDATE OF last_used_at ON sessions \
             FOR EACH ROW EXECUTE FUNCTION fail_touch()",
        ],
    };
    for sql in statements {
        db.conn.execute_unprepared(sql).await.unwrap();
    }
}

#[tokio::test]
async fn a_failing_touch_is_logged_and_the_request_allowed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, stamp) = env.login("alice").await;
        let (session, token) = env
            .sessions
            .create_client(user_id, &stamp, &device(), None)
            .await
            .unwrap();
        break_touches(db).await;
        env.clock.advance(MINUTE);
        let before = env.queries();

        let from_db = env.auth(&token).await;
        let from_cache = env.auth(&token).await;
        let attempts = env.queries() - before;

        assert_eq!(from_db.unwrap(), user_id);
        assert_eq!(from_cache.unwrap(), user_id);
        // SELECT + failed UPDATE, then (still not recorded) another failed UPDATE.
        assert_eq!(attempts, 3);
        let stored = row(db, session.id).await.unwrap();
        assert_eq!(stored.last_used_at, session.last_used_at);
    })
    .await;
}

#[tokio::test]
async fn a_touch_that_finds_the_row_revoked_rejects_the_cached_token() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;
        // Cached at +59s; revoked directly (no invalidation); touched at +60s.
        env.clock.advance(Duration::from_secs(59));
        env.auth(&token).await.unwrap();
        session::Entity::update_many()
            .col_expr(session::Column::RevokedAt, Expr::value(env.clock.now()))
            .exec(&db.conn)
            .await
            .unwrap();
        env.clock.advance(Duration::from_secs(1));

        let result = env.auth(&token).await;

        assert!(matches!(result, Err(SessionError::Revoked)), "{result:?}");
        assert!(env.sessions.cache().is_empty());
    })
    .await;
}

#[tokio::test]
async fn user_timestamps_come_from_the_injected_clock() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, _token) = env.client_session("alice").await;
        env.clock.advance(DAY);
        let changed_at = env.clock.now();

        env.users
            .set_password(alice, &Password::new("another-Phrase-7"), false)
            .await
            .unwrap();

        let stored = user::Entity::find_by_id(alice)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let revoked = session::Entity::find()
            .filter(session::Column::UserId.eq(alice))
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.password_changed_at, Some(changed_at));
        assert_eq!(stored.updated_at, changed_at);
        assert_eq!(revoked.revoked_at, Some(changed_at));
    })
    .await;
}

#[tokio::test]
async fn the_event_consumer_revokes_every_session_of_the_user() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        let (bob, bob_token) = env.client_session("bob").await;
        env.auth(&token).await.unwrap();
        env.auth(&bob_token).await.unwrap();
        let (events, receiver) = broadcast::channel(8);

        events.send(UserEvent::Disabled { user_id: alice }).unwrap();
        drop(events);
        env.sessions.spawn_event_consumer(receiver).await.unwrap();

        assert!(matches!(env.auth(&token).await, Err(SessionError::Revoked)));
        assert_eq!(env.auth(&bob_token).await.unwrap(), bob);
    })
    .await;
}

/// The password change revokes the old sessions in its own transaction and the user signs
/// in again right after; the event consumer runs later and must not revoke that new session.
#[tokio::test]
async fn a_late_password_change_event_spares_sessions_created_after_the_change() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, old) = env.client_session("alice").await;
        env.clock.advance(MINUTE);
        let new_password = Password::new("another-Phrase-7");
        env.users
            .set_password(alice, &new_password, false)
            .await
            .unwrap();
        env.clock.advance(MINUTE);
        let login = env
            .users
            .authenticate_local("alice", &new_password)
            .await
            .unwrap();
        let (_, fresh) = env
            .sessions
            .create_client(alice, &login.stamp, &device(), None)
            .await
            .unwrap();
        let (events, receiver) = broadcast::channel(8);

        events
            .send(UserEvent::PasswordChanged { user_id: alice })
            .unwrap();
        drop(events);
        env.sessions.spawn_event_consumer(receiver).await.unwrap();

        assert!(matches!(env.auth(&old).await, Err(SessionError::Revoked)));
        assert_eq!(env.auth(&fresh).await.unwrap(), alice);
    })
    .await;
}

#[tokio::test]
async fn lost_events_flush_the_whole_cache() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();
        let (events, receiver) = broadcast::channel(1);
        // Overflow a 1-slot channel before the consumer runs: it starts lagged.
        for user_id in [900, 901, 902] {
            events.send(UserEvent::Disabled { user_id }).unwrap();
        }
        drop(events);

        env.sessions.spawn_event_consumer(receiver).await.unwrap();

        assert!(env.sessions.cache().is_empty());
        // Flushed, not revoked: the session is re-read from the database.
        assert_eq!(env.auth(&token).await.unwrap(), alice);
    })
    .await;
}

#[tokio::test]
async fn list_shows_only_usable_sessions_most_recent_first() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, stamp) = env.login("alice").await;
        let (older, _) = env
            .sessions
            .create_console(alice, &stamp, None, None)
            .await
            .unwrap();
        env.clock.advance(MINUTE);
        let (newer, _) = env
            .sessions
            .create_client(alice, &stamp, &device(), None)
            .await
            .unwrap();
        let (revoked, _) = env
            .sessions
            .create_console(alice, &stamp, None, None)
            .await
            .unwrap();
        env.sessions.revoke(revoked.id).await.unwrap();
        env.client_session("bob").await;

        let listed: Vec<i64> = env
            .sessions
            .list_for_user(alice)
            .await
            .unwrap()
            .iter()
            .map(|s| s.id)
            .collect();
        env.clock.advance(IDLE_TTL);
        let after_idle = env.sessions.list_for_user(alice).await.unwrap();

        assert_eq!(listed, [newer.id, older.id]);
        assert!(after_idle.is_empty());
    })
    .await;
}

#[tokio::test]
async fn cleanup_removes_only_revoked_and_expired_sessions() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, stamp) = env.login("alice").await;
        let console = async || {
            env.sessions
                .create_console(alice, &stamp, None, None)
                .await
                .unwrap()
                .0
        };
        let idle = console().await;
        env.clock.advance(IDLE_TTL);
        let live = console().await;
        let revoked = console().await;
        env.sessions.revoke(revoked.id).await.unwrap();
        let expired = console().await;
        session::Entity::update_many()
            .col_expr(session::Column::ExpiresAt, Expr::value(env.clock.now()))
            .filter(session::Column::Id.eq(expired.id))
            .exec(&db.conn)
            .await
            .unwrap();

        let deleted = env.sessions.cleanup().await.unwrap();
        let again = env.sessions.cleanup().await.unwrap();

        assert_eq!(deleted, 3);
        assert_eq!(again, 0);
        assert!(row(db, live.id).await.is_some());
        for gone in [idle.id, revoked.id, expired.id] {
            assert!(row(db, gone).await.is_none(), "session {gone} survived");
        }
    })
    .await;
}

fn hit(result: CachedAuth) -> Result<i64, SessionError> {
    match result {
        CachedAuth::Hit(outcome) => outcome.map(|p| p.user_id),
        CachedAuth::Miss => panic!("expected a cache hit"),
    }
}

#[tokio::test]
async fn authenticate_cached_answers_from_the_cache_only() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (alice, token) = env.client_session("alice").await;
        let sessions = &env.sessions;

        let before = env.queries();
        let fresh = sessions
            .authenticate_cached(token.expose(), SessionKind::Client)
            .await;
        let malformed = sessions
            .authenticate_cached("not-a-token", SessionKind::Client)
            .await;
        assert!(matches!(fresh, CachedAuth::Miss), "{}", db.backend);
        assert!(matches!(hit(malformed), Err(SessionError::InvalidToken)));
        assert_eq!(env.queries(), before, "no database access");

        env.auth(&token).await.unwrap();
        let before = env.queries();
        let valid = sessions
            .authenticate_cached(token.expose(), SessionKind::Client)
            .await;
        let wrong_kind = sessions
            .authenticate_cached(token.expose(), SessionKind::Console)
            .await;
        assert_eq!(hit(valid).unwrap(), alice);
        assert!(matches!(hit(wrong_kind), Err(SessionError::InvalidToken)));
        assert_eq!(
            env.queries(),
            before,
            "hits are served without the database"
        );

        // Past the cache TTL the entry no longer answers: back to the database.
        env.clock.advance(SESSION_TTL);
        let stale = sessions
            .authenticate_cached(token.expose(), SessionKind::Client)
            .await;
        assert!(matches!(stale, CachedAuth::Miss));
    })
    .await;
}

#[tokio::test]
async fn authenticate_cached_misses_after_revocation() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, token) = env.client_session("alice").await;
        env.auth(&token).await.unwrap();

        assert!(env.sessions.revoke_token(token.expose()).await.unwrap());

        let after = env
            .sessions
            .authenticate_cached(token.expose(), SessionKind::Client)
            .await;
        assert!(matches!(after, CachedAuth::Miss), "{}", db.backend);
    })
    .await;
}
