//! The real client API, in process: a fresh data_dir and SQLite database
//! (migrated, default settings), the production `client_api` router and the
//! production middleware stack, called through `tower::ServiceExt::oneshot`.

use std::io::Read;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, header};
use deskhaven_config::Cidr;
use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_config::settings::{Change, PinSet};
use deskhaven_core::clock::{SharedClock, SystemClock};
use deskhaven_core::crypto::{MASTER_KEY_LEN, MasterKey, SecretBox};
use deskhaven_core::ldap::{Connector, Ldap3Connector, LdapService};
use deskhaven_core::sessions::SessionService;
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::users::{PasswordHasher, UserService};
use deskhaven_http::{ErrorBodies, apply_middleware, redirect_client_root, single_listener_router};
use flate2::read::GzDecoder;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::check::Reply;
use crate::fixture::{self, Listen};

/// TCP peer of a fixture that names none (TEST-NET-3, not a trusted proxy).
pub const DEFAULT_PEER: SocketAddr = SocketAddr::new(
    std::net::IpAddr::V4(std::net::Ipv4Addr::new(203, 0, 113, 7)),
    50_000,
);
/// Larger replies are a failure in themselves.
const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
const DRAIN: Duration = Duration::from_secs(1);
const TEST_MASTER_KEY: [u8; MASTER_KEY_LEN] = [0x42; MASTER_KEY_LEN];
const TEST_SIGNAL_PUBLIC_KEY: &str = "contract-test-signal-public-key";

pub struct TestApp {
    pub state: Arc<AppState>,
    router: Router,
    _shutdown: CancellationToken,
    _data_dir: TempDir,
}

/// How a [`TestApp`] differs from production; the default is production.
#[derive(Default)]
pub struct TestOptions {
    /// Merged into the client API (test-only routes, same state and middleware).
    pub routes: Router<Arc<AppState>>,
    /// Users and sessions rebuilt around this hasher (e.g. one slot, to provoke
    /// `Busy`). Keep a clone: clones share its concurrency limit.
    pub hasher: Option<PasswordHasher>,
    /// `security.trusted_proxies` (a peer inside without `X-Forwarded-For` is a
    /// proxy-fallback client).
    pub trusted_proxies: Vec<Cidr>,
    /// Which listener the requests go to (default: the client API alone).
    pub listen: Listen,
    /// Bootstrap `public_url` (default unset: OIDC logins are refused).
    pub public_url: String,
    /// Every service rebuilt on this clock (default: the system clock), e.g. a
    /// `ManualClock` to step past rate-limit intervals. Not combined with `hasher`.
    pub clock: Option<SharedClock>,
    /// The LDAP service talks to this directory (a fake) instead of a real server.
    pub ldap: Option<Arc<dyn Connector>>,
}

impl TestApp {
    /// Production routes only.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_options(TestOptions::default()).await
    }

    /// Production routes plus `extra` (test-only routes; they get the same
    /// state and sit behind the same middleware).
    pub async fn start_with(extra: Router<Arc<AppState>>) -> anyhow::Result<Self> {
        Self::start_with_options(TestOptions {
            routes: extra,
            ..TestOptions::default()
        })
        .await
    }

    /// Production routes, with the user and session services rebuilt around
    /// `hasher`.
    pub async fn start_with_hasher(hasher: PasswordHasher) -> anyhow::Result<Self> {
        Self::start_with_options(TestOptions {
            hasher: Some(hasher),
            ..TestOptions::default()
        })
        .await
    }

    pub async fn start_with_options(options: TestOptions) -> anyhow::Result<Self> {
        let TestOptions {
            routes: extra,
            hasher,
            trusted_proxies,
            listen,
            public_url,
            clock,
            ldap,
        } = options;
        let data_dir = tempfile::tempdir().context("cannot create a temporary data_dir")?;
        let mut config = Bootstrap {
            data_dir: data_dir.path().to_path_buf(),
            public_url,
            ..Bootstrap::default()
        };
        config.security.trusted_proxies = trusted_proxies;
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&TEST_MASTER_KEY)));
        let base = deskhaven::startup::open_state(
            &config,
            PinSet::empty(),
            crypto,
            TEST_SIGNAL_PUBLIC_KEY.to_owned(),
        )
        .await?;
        let state = match (hasher, clock) {
            (Some(hasher), _) => with_hasher(base, hasher),
            (None, Some(clock)) => on_clock(&base, clock),
            (None, None) => base,
        };
        // One instance: the routers and the seeds share it.
        let state = Arc::new(match ldap {
            Some(connector) => with_ldap(state, connector),
            None => state,
        });
        let shutdown = CancellationToken::new();
        let apps = deskhaven::http::apps(
            Arc::clone(&state),
            deskhaven_admin_api::api_routes()?
                .trusted_origins(deskhaven::http::trusted_origins(&config)),
            &shutdown,
            None,
        );
        let client_api = apps.client_api.merge(extra.with_state(Arc::clone(&state)));
        let http = deskhaven::http::http_config(&config, DRAIN);
        let (client_api, errors) = match listen {
            // As served on a split client_api listener (deskhaven_http::serve);
            // single-listener mode routes `/api/*` identically.
            Listen::ClientApi => (client_api, ErrorBodies::uniform(apps.errors.client_api)),
            Listen::Single => (single_listener_router(client_api, apps.admin), apps.errors),
            Listen::Split { admin_public_url } => (
                redirect_client_root(client_api, &admin_public_url)?,
                ErrorBodies::uniform(apps.errors.client_api),
            ),
        };
        let router = apply_middleware(client_api, &http, errors);
        Ok(Self {
            state,
            router,
            _shutdown: shutdown,
            _data_dir: data_dir,
        })
    }

    /// Changes one runtime setting through the real settings service (hot
    /// reload: every service sees it on its next read).
    pub async fn set_setting(&self, key: &str, value: serde_json::Value) -> anyhow::Result<()> {
        let settings = &self.state.settings;
        let version = settings
            .snapshot()
            .view(key)
            .with_context(|| format!("unknown setting {key:?}"))?
            .version;
        let change = Change {
            key: key.to_owned(),
            value,
            version,
        };
        settings
            .update(UpdateRequest {
                changes: vec![change],
                actor: None,
                reason: None,
            })
            .await?;
        Ok(())
    }

    /// Sends `request` and returns what a client's HTTP stack hands its caller: a body the
    /// server compressed (`Content-Encoding: gzip`, announced by the request's
    /// `Accept-Encoding`) is decompressed, as Dart's `HttpClient` does; the headers stay as
    /// the server sent them, so fixtures can still assert the encoding.
    pub async fn send(&self, request: &fixture::Request) -> anyhow::Result<Reply> {
        let response = self.router.clone().oneshot(build(request)?).await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), MAX_REPLY_BYTES)
            .await
            .context("cannot read the response body")?;
        Ok(Reply {
            status,
            body: decode(&headers, body.to_vec())?,
            headers,
        })
    }
}

/// `body` after undoing its `Content-Encoding` (only `gzip` is understood; anything else
/// is an error, so a fixture never compares compressed bytes by accident).
fn decode(headers: &HeaderMap, body: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    let Some(encoding) = headers.get(header::CONTENT_ENCODING) else {
        return Ok(body);
    };
    anyhow::ensure!(
        encoding.as_bytes().eq_ignore_ascii_case(b"gzip"),
        "unsupported content-encoding {encoding:?}"
    );
    let mut plain = Vec::new();
    let limit = MAX_REPLY_BYTES as u64 + 1;
    GzDecoder::new(body.as_slice())
        .take(limit)
        .read_to_end(&mut plain)
        .context("the response is not valid gzip")?;
    anyhow::ensure!(
        plain.len() <= MAX_REPLY_BYTES,
        "the decompressed response is too large"
    );
    Ok(plain)
}

/// `base` with users and sessions on `hasher` (same database, settings and
/// throttle; the clock is the production one, as in `open_state`).
/// A state like `base` (same database, settings and keys) whose services run on `clock`.
fn on_clock(base: &AppState, clock: SharedClock) -> AppState {
    AppState::new(
        base.db.clone(),
        base.settings.clone(),
        Arc::clone(&base.crypto),
        base.signal_public_key.clone(),
        PasswordHasher::default(),
        clock,
        &StateConfig {
            public_url: base.public_url.clone(),
            console_url: base.console_url.clone(),
            signal_port: base.signal_port,
        },
    )
}

fn with_hasher(base: AppState, hasher: PasswordHasher) -> AppState {
    let clock: SharedClock = Arc::new(SystemClock);
    let users = UserService::with_clock(
        base.db.clone(),
        base.settings.clone(),
        hasher,
        Arc::clone(&clock),
    );
    let sessions = SessionService::new(
        base.db.clone(),
        base.settings.clone(),
        users.clone(),
        Arc::clone(&clock),
    );
    // Password logins go through the LDAP service, which must use the same users.
    let ldap = ldap_service(&base, &users, clock, Arc::new(Ldap3Connector::new()));
    AppState {
        users,
        sessions,
        ldap,
        ..base
    }
}

/// `base` whose LDAP service talks to `connector`.
fn with_ldap(base: AppState, connector: Arc<dyn Connector>) -> AppState {
    let ldap = ldap_service(&base, &base.users, Arc::clone(&base.clock), connector);
    AppState { ldap, ..base }
}

fn ldap_service(
    base: &AppState,
    users: &UserService,
    clock: SharedClock,
    connector: Arc<dyn Connector>,
) -> LdapService {
    LdapService::new(
        base.db.clone(),
        Arc::clone(&base.crypto),
        users.clone(),
        base.groups.clone(),
        clock,
        connector,
    )
}

fn build(request: &fixture::Request) -> anyhow::Result<Request<Body>> {
    let method = Method::from_bytes(request.method.as_bytes())?;
    let body = match &request.body {
        fixture::Body::Empty => Vec::new(),
        fixture::Body::Json(value) => serde_json::to_vec(value)?,
        fixture::Body::Text(text) => text.clone().into_bytes(),
    };
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(request.path.as_str())
        .extension(ConnectInfo(request.peer.unwrap_or(DEFAULT_PEER)));
    for (name, value) in &request.headers {
        builder = builder.header(
            HeaderName::from_bytes(name.as_bytes())?,
            HeaderValue::from_str(value)?,
        );
    }
    let has_length = request
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(header::CONTENT_LENGTH.as_str()));
    let carries_body = !body.is_empty() || ![Method::GET, Method::HEAD].contains(&method);
    if carries_body && !has_length {
        builder = builder.header(header::CONTENT_LENGTH, body.len());
    }
    Ok(builder.body(Body::from(body))?)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::GzEncoder;

    use super::*;

    fn gzip(plain: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(plain).unwrap();
        encoder.finish().unwrap()
    }

    fn encoded(value: &'static str) -> HeaderMap {
        HeaderMap::from_iter([(header::CONTENT_ENCODING, HeaderValue::from_static(value))])
    }

    #[test]
    fn gzip_bodies_are_decompressed_and_others_pass_through() {
        let plain = decode(&encoded("GZip"), gzip(b"{\"a\":1}")).unwrap();
        let untouched = decode(&HeaderMap::new(), b"raw".to_vec()).unwrap();

        assert_eq!(plain, b"{\"a\":1}");
        assert_eq!(untouched, b"raw");
    }

    #[test]
    fn unknown_encodings_and_broken_gzip_are_errors() {
        assert!(decode(&encoded("br"), b"x".to_vec()).is_err());
        assert!(decode(&encoded("gzip"), b"not gzip".to_vec()).is_err());
    }
}
