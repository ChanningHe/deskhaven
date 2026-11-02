//! The real client API, in process: a fresh data_dir and SQLite database
//! (migrated, default settings), the production `client_api` router and the
//! production middleware stack, called through `tower::ServiceExt::oneshot`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{HeaderName, HeaderValue, Method, Request, header};
use deskhaven_config::bootstrap::Bootstrap;
use deskhaven_config::settings::PinSet;
use deskhaven_core::crypto::{MASTER_KEY_LEN, MasterKey, SecretBox};
use deskhaven_core::state::AppState;
use deskhaven_http::{ErrorBodies, apply_middleware};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::check::Reply;
use crate::fixture;

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

impl TestApp {
    /// Production routes only.
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with(Router::new()).await
    }

    /// Production routes plus `extra` (test-only routes; they get the same
    /// state and sit behind the same middleware).
    pub async fn start_with(extra: Router<Arc<AppState>>) -> anyhow::Result<Self> {
        let data_dir = tempfile::tempdir().context("cannot create a temporary data_dir")?;
        let config = Bootstrap {
            data_dir: data_dir.path().to_path_buf(),
            ..Bootstrap::default()
        };
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&TEST_MASTER_KEY)));
        // One instance: the routers and the seeds share it.
        let state = Arc::new(
            deskhaven::startup::open_state(
                &config,
                PinSet::empty(),
                crypto,
                TEST_SIGNAL_PUBLIC_KEY.to_owned(),
            )
            .await?,
        );
        let shutdown = CancellationToken::new();
        let apps = deskhaven::http::apps(Arc::clone(&state), &shutdown);
        let client_api = apps.client_api.merge(extra.with_state(Arc::clone(&state)));
        // As served on a split client_api listener (deskhaven_http::serve);
        // single-listener mode routes `/api/*` identically.
        let http = deskhaven::http::http_config(&config, DRAIN);
        let router = apply_middleware(
            client_api,
            &http,
            ErrorBodies::uniform(apps.errors.client_api),
        );
        Ok(Self {
            state,
            router,
            _shutdown: shutdown,
            _data_dir: data_dir,
        })
    }

    pub async fn send(&self, request: &fixture::Request) -> anyhow::Result<Reply> {
        let response = self.router.clone().oneshot(build(request)?).await?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = to_bytes(response.into_body(), MAX_REPLY_BYTES)
            .await
            .context("cannot read the response body")?;
        Ok(Reply {
            status,
            headers,
            body: body.to_vec(),
        })
    }
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
