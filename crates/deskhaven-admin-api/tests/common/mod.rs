//! Admin API harness: real state on a dual-backend test database, the given routes behind
//! the production middleware stack, and console sessions minted through the real login
//! services (password check + `SessionService::create_console`).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::{Extension, Router};
use deskhaven_admin_api::{AdminRoutes, DetachedAudits, SESSION_COOKIE, csrf_token};

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::{SharedClock, SystemClock};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::oidc::{FetchError, FetchFuture, HttpFetch};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::settings::SettingsService;
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::update::UpdateService;
use deskhaven_core::users::{Credential, HashParams, NewUser, Password, PasswordHasher, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::TestDb;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use openidconnect::HttpRequest;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;
use tower::ServiceExt;

pub mod audit_rows;
pub mod client;
pub mod devices;
pub mod login;
pub mod own;
pub mod probe;
pub mod tokens;
pub mod web_client;

pub const PASSWORD: &str = "correct horse battery staple 42";
/// Cheap Argon2id parameters: tests create many accounts.
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};
/// `deskhaven_admin_api::CSRF_HEADER` as a static string.
pub const CSRF_HEADER_NAME: &str = "x-deskhaven-csrf";
/// TEST-NET-3 peer, not a trusted proxy.
pub const PEER: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 9)),
    40_000,
);

pub struct Harness {
    pub state: Arc<AppState>,
    router: Router,
    /// The gate's detached fallback audit writes, not yet awaited.
    detached: Mutex<UnboundedReceiver<JoinHandle<()>>>,
}

/// A logged-in console user: the `Cookie` header value and the matching CSRF token.
#[derive(Clone, Debug)]
pub struct Console {
    pub user_id: i64,
    pub cookie: String,
    pub csrf: String,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

/// The production routes plus `extra`, on `db`.
pub async fn harness(db: &TestDb, extra: AdminRoutes) -> Harness {
    harness_pinned(db, extra, PinSet::empty()).await
}

/// [`harness`] with settings pinned by the bootstrap configuration.
pub async fn harness_pinned(db: &TestDb, extra: AdminRoutes, pins: PinSet) -> Harness {
    build(db, extra, pins, Arc::new(OfflineFetch)).await
}

/// [`harness`] whose update check asks `fetch`.
pub async fn harness_with_updates(db: &TestDb, fetch: Arc<dyn HttpFetch>) -> Harness {
    build(db, AdminRoutes::new(), PinSet::empty(), fetch).await
}

/// The production routes with every service on `clock` (e.g. a `ManualClock`).
pub async fn harness_on_clock(db: &TestDb, pins: PinSet, clock: SharedClock) -> Harness {
    build_on(db, AdminRoutes::new(), pins, Arc::new(OfflineFetch), clock).await
}

async fn build(
    db: &TestDb,
    extra: AdminRoutes,
    pins: PinSet,
    update_fetch: Arc<dyn HttpFetch>,
) -> Harness {
    build_on(db, extra, pins, update_fetch, Arc::new(SystemClock)).await
}

async fn build_on(
    db: &TestDb,
    extra: AdminRoutes,
    pins: PinSet,
    update_fetch: Arc<dyn HttpFetch>,
    clock: SharedClock,
) -> Harness {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[9; 32])));
    let (settings, warnings) = SettingsService::load(db.conn.clone(), Arc::clone(&crypto), pins)
        .await
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let state = AppState::new(
        db.conn.clone(),
        settings,
        crypto,
        "test-signal-key".to_owned(),
        PasswordHasher::new(FAST).unwrap(),
        clock,
        &StateConfig::default(),
    );
    let state = Arc::new(with_update_fetch(state, update_fetch));
    let routes = deskhaven_admin_api::api_routes()
        .unwrap()
        .merge(extra)
        .unwrap();
    let router = apply_middleware(
        routes.into_router(Arc::clone(&state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_admin_api::render_stack_error),
    );
    Harness::new(state, router)
}

/// `state` whose update check asks `fetch` instead of GitHub.
pub fn with_update_fetch(state: AppState, fetch: Arc<dyn HttpFetch>) -> AppState {
    let updates = UpdateService::new(state.settings.clone(), Arc::clone(&state.clock), fetch);
    AppState {
        updates: Arc::new(updates),
        ..state
    }
}

/// `state` whose update check never leaves the process.
pub fn offline(state: AppState) -> AppState {
    with_update_fetch(state, Arc::new(OfflineFetch))
}

/// Refuses every request: harness tests never reach GitHub.
pub struct OfflineFetch;

impl HttpFetch for OfflineFetch {
    fn fetch(&self, _request: HttpRequest) -> FetchFuture<'_> {
        Box::pin(async { Err(FetchError::Refused("tests stay offline")) })
    }
}

/// Answers every request with a release tagged `tag`; counts the requests.
pub struct FixedRelease {
    tag: &'static str,
    calls: AtomicUsize,
}

impl FixedRelease {
    pub fn new(tag: &'static str) -> Self {
        Self {
            tag,
            calls: AtomicUsize::new(0),
        }
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl HttpFetch for FixedRelease {
    fn fetch(&self, _request: HttpRequest) -> FetchFuture<'_> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let body = serde_json::json!({ "tag_name": self.tag }).to_string();
        Box::pin(async move { Ok(openidconnect::http::Response::new(body.into_bytes())) })
    }
}

impl Harness {
    /// `router` on `state`, its fallback audit writes handed to the harness to await.
    pub fn new(state: Arc<AppState>, router: Router) -> Self {
        let (hook, detached) = unbounded_channel();
        Self {
            state,
            router: router.layer(Extension(DetachedAudits(hook))),
            detached: Mutex::new(detached),
        }
    }

    /// Awaits the fallback audit writes of every request so far (the gate writes them in
    /// detached tasks), so `console_audit` reads that follow see their rows.
    pub async fn settle(&self) {
        loop {
            let next = self.detached.lock().unwrap().try_recv();
            let Ok(task) = next else { return };
            task.await.unwrap();
        }
    }

    /// The address book service over the harness state, to seed and inspect books directly.
    pub fn books(&self) -> deskhaven_core::addressbooks::AddressBookService {
        deskhaven_core::addressbooks::AddressBookService::new(
            self.state.db.clone(),
            self.state.settings.clone(),
            Arc::clone(&self.state.crypto),
        )
    }

    /// Creates an active local user of `role` and logs it in to the console.
    pub async fn console(&self, username: &str, role: Role) -> Console {
        let user_id = self.create_user(username, role).await;
        let auth = self
            .state
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap();
        let (_, token) = self
            .state
            .sessions
            .create_console(user_id, &auth.stamp, Some(PEER.ip()), Some("test-agent"))
            .await
            .unwrap();
        Console {
            user_id,
            cookie: format!("{SESSION_COOKIE}={}", token.expose()),
            csrf: csrf_token(token.expose()),
        }
    }

    /// A *client* session of a new user, presented as the console cookie.
    pub async fn client_session_cookie(&self, username: &str) -> String {
        let user_id = self.create_user(username, Role::Owner).await;
        let auth = self
            .state
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap();
        let device = ClientDevice {
            rustdesk_id: "123456789".to_owned(),
            uuid: "uuid-1".to_owned(),
            info: None,
        };
        let (_, token) = self
            .state
            .sessions
            .create_client(user_id, &auth.stamp, &device, None)
            .await
            .unwrap();
        format!("{SESSION_COOKIE}={}", token.expose())
    }

    /// Creates an active local user of `role` (password [`PASSWORD`]); returns its id.
    pub async fn create_user(&self, username: &str, role: Role) -> i64 {
        let credential = Credential::Local {
            password: Password::new(PASSWORD),
        };
        let new = NewUser {
            role,
            ..NewUser::new(username, credential)
        };
        self.state.users.create(new).await.unwrap().id
    }

    /// Sends `method path` with optional session cookie, CSRF header and JSON body.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        csrf: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        let mut headers = Vec::new();
        if let Some(cookie) = cookie {
            headers.push(("cookie", cookie));
        }
        if let Some(csrf) = csrf {
            headers.push((CSRF_HEADER_NAME, csrf));
        }
        self.raw(request(method, path, &headers, body)).await
    }

    /// The router's response future for `request` (to drop it mid-flight).
    pub fn call(
        &self,
        request: Request<Body>,
    ) -> impl std::future::Future<Output = Result<axum::response::Response, std::convert::Infallible>>
    {
        self.router.clone().oneshot(request)
    }

    /// GET with the console's cookie.
    pub async fn get(&self, path: &str, console: &Console) -> Reply {
        self.send(Method::GET, path, Some(&console.cookie), None, None)
            .await
    }

    /// A state-changing request with the console's cookie and CSRF token.
    pub async fn write(&self, method: Method, path: &str, console: &Console, body: Value) -> Reply {
        let (cookie, csrf) = (Some(console.cookie.as_str()), Some(console.csrf.as_str()));
        self.send(method, path, cookie, csrf, Some(body)).await
    }

    /// A body-less state-changing request (`POST` action, `DELETE`) as `console`.
    pub async fn act(&self, method: Method, path: &str, console: &Console) -> Reply {
        let (cookie, csrf) = (Some(console.cookie.as_str()), Some(console.csrf.as_str()));
        self.send(method, path, cookie, csrf, None).await
    }

    /// Every console audit row, oldest first.
    pub async fn audit_rows(&self) -> Vec<console_audit::Model> {
        use sea_orm::{EntityTrait, QueryOrder};
        self.settle().await;
        console_audit::Entity::find()
            .order_by_asc(console_audit::Column::Id)
            .all(&self.state.db)
            .await
            .unwrap()
    }

    /// Sends `request`; returns once its fallback audit row, if any, is written.
    pub async fn raw(&self, request: Request<Body>) -> Reply {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        self.settle().await;
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| panic!("non-JSON body: {}", String::from_utf8_lossy(&bytes)))
        };
        Reply {
            status,
            headers,
            body,
        }
    }
}

/// `method path` from [`PEER`] with `headers` and an optional JSON body.
pub fn request(
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo(PEER));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let body = match body {
        Some(json) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&json).unwrap())
        }
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

/// `(cookie, csrf)` headers of `console`, followed by `extra`.
pub fn auth_headers<'a>(
    console: &'a Console,
    extra: &[(&'a str, &'a str)],
) -> Vec<(&'a str, &'a str)> {
    let mut headers = vec![
        ("cookie", console.cookie.as_str()),
        (CSRF_HEADER_NAME, console.csrf.as_str()),
    ];
    headers.extend_from_slice(extra);
    headers
}

impl Reply {
    pub fn code(&self) -> &str {
        self.body["error"]["code"].as_str().unwrap_or("")
    }
}
