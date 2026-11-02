//! Bind the client API listener (always) and the admin listener (when split),
//! serve both with `ConnectInfo<SocketAddr>` and shut down gracefully.

use std::io;
use std::net::SocketAddr;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use url::Url;

use crate::config::{ConfigError, HttpConfig};
use crate::conn::serve_listener;
use crate::middleware::apply_middleware;
use crate::nofile::{self, Share};
use crate::paths;
use crate::stack_error::ErrorBodies;

const CLIENT_API: &str = "client_api";
const ADMIN: &str = "admin";

/// Listener topology, already resolved by the caller (e.g. from
/// `deskhaven_config::bootstrap::Listen::admin_addr()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenConfig {
    pub client_api: SocketAddr,
    /// `Some` = admin console/API on its own listener.
    pub admin: Option<SocketAddr>,
    /// Split mode only: client_api `GET /` redirects (302) here; `None` = 404.
    /// Must be an absolute http(s) URL. Ignored (with a warning) when not split.
    pub admin_public_url: Option<String>,
}

/// Bare routers (no middleware): [`Listeners::serve`] applies the stack.
///
/// Single listener: requests not matched by `client_api` fall through to
/// `admin`, so a path defined in both is served by `client_api` (any fallback
/// on `client_api` is replaced), except under `/api`: an unmatched `/api/*`
/// path is a 404 rendered by the client renderer and never reaches `admin`.
/// Keep the APIs on disjoint prefixes: `/api/*` (client) and `/admin/api/*` +
/// console (admin). The SPA fallback of the admin router (deskhaven-web) must
/// exclude both API prefixes so unknown API paths keep their JSON 404.
/// Split: each router is served on its own listener.
pub struct Apps {
    pub client_api: Router,
    pub admin: Router,
    /// Bodies for stack-generated errors; only paths under the API prefixes are
    /// re-rendered (see [`ErrorBodies`]). Single listener: `/api/*` uses
    /// `client_api`, `/admin/api/*` uses `admin`. Split: each listener uses its
    /// own API's renderer for both prefixes.
    pub errors: ErrorBodies,
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error("cannot bind {name} listener on {addr}")]
    Bind {
        name: &'static str,
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
    #[error("listen.admin_public_url must be an absolute http(s) URL")]
    InvalidAdminPublicUrl,
    #[error(transparent)]
    InvalidConfig(#[from] ConfigError),
}

struct Bound {
    listener: TcpListener,
    addr: SocketAddr,
}

async fn bind_one(name: &'static str, addr: SocketAddr) -> Result<Bound, ServeError> {
    let bind_err = |source| ServeError::Bind { name, addr, source };
    let listener = TcpListener::bind(addr).await.map_err(bind_err)?;
    let addr = listener.local_addr().map_err(bind_err)?;
    Ok(Bound { listener, addr })
}

struct AdminListener {
    bound: Bound,
    redirect: Option<HeaderValue>,
}

pub struct Listeners {
    client_api: Bound,
    admin: Option<AdminListener>,
}

impl Listeners {
    pub async fn bind(cfg: &ListenConfig) -> Result<Self, ServeError> {
        let url = cfg.admin_public_url.as_deref().filter(|u| !u.is_empty());
        let admin = match cfg.admin {
            None => {
                if url.is_some() {
                    tracing::warn!("listen.admin_public_url is ignored: admin is not split");
                }
                None
            }
            Some(addr) => Some((addr, redirect_target(url)?)),
        };
        let client_api = bind_one(CLIENT_API, cfg.client_api).await?;
        let admin = match admin {
            None => None,
            Some((addr, redirect)) => Some(AdminListener {
                bound: bind_one(ADMIN, addr).await?,
                redirect,
            }),
        };
        Ok(Self { client_api, admin })
    }

    /// Actual bound address (resolves port 0).
    pub fn client_api_addr(&self) -> SocketAddr {
        self.client_api.addr
    }

    pub fn admin_addr(&self) -> Option<SocketAddr> {
        self.admin.as_ref().map(|a| a.bound.addr)
    }

    /// Validate `cfg`, then serve until `shutdown` is cancelled and drain (see
    /// [`HttpConfig`]). Accept errors are logged and retried, so after a valid
    /// config this only returns on shutdown.
    pub async fn serve(
        self,
        apps: Apps,
        cfg: &HttpConfig,
        shutdown: CancellationToken,
    ) -> Result<(), ServeError> {
        cfg.validate()?;
        let Some(admin) = self.admin else {
            let cfg = &fit_descriptor_budget(cfg, Share::Single);
            let router = single_listener_router(apps.client_api, apps.admin);
            let router = apply_middleware(router, cfg, apps.errors);
            serve_listener(CLIENT_API, self.client_api.listener, router, cfg, shutdown).await;
            return Ok(());
        };
        let client_cfg = &fit_descriptor_budget(cfg, Share::SplitClientApi);
        let admin_cfg = &HttpConfig {
            strict_transport_security: cfg.admin_strict_transport_security.clone(),
            ..fit_descriptor_budget(cfg, Share::SplitAdmin)
        };
        let client_router = match admin.redirect {
            Some(location) => with_root_redirect(apps.client_api, location),
            None => apps.client_api,
        };
        let client_errors = ErrorBodies::uniform(apps.errors.client_api);
        let admin_errors = ErrorBodies::uniform(apps.errors.admin);
        tokio::join!(
            serve_listener(
                CLIENT_API,
                self.client_api.listener,
                apply_middleware(client_router, client_cfg, client_errors),
                client_cfg,
                shutdown.clone(),
            ),
            serve_listener(
                ADMIN,
                admin.bound.listener,
                apply_middleware(apps.admin, admin_cfg, admin_errors),
                admin_cfg,
                shutdown,
            ),
        );
        Ok(())
    }
}

/// `cfg` with `max_connections` clamped to this listener's `RLIMIT_NOFILE` share.
fn fit_descriptor_budget(cfg: &HttpConfig, share: Share) -> HttpConfig {
    let soft = nofile::current_soft_limit();
    let max_connections =
        nofile::connection_budget(cfg.max_connections, soft, cfg.nofile_budget_percent, share);
    if max_connections < cfg.max_connections {
        tracing::warn!(
            ?share,
            configured = cfg.max_connections,
            effective = max_connections,
            nofile = soft,
            "max_connections clamped to fit RLIMIT_NOFILE; raise the limit to serve more"
        );
    }
    HttpConfig {
        max_connections,
        ..cfg.clone()
    }
}

/// The routers as one listener serves them: `client_api` first; unmatched
/// paths go to `admin`, except under `/api`.
pub fn single_listener_router(client_api: Router, admin: Router) -> Router {
    client_api.fallback(move |req: Request| {
        let admin = admin.clone();
        async move {
            if paths::is_client_api(req.uri().path()) {
                return StatusCode::NOT_FOUND.into_response();
            }
            admin.oneshot(req).await.into_response()
        }
    })
}

/// An absolute http(s) URL that is also a valid header value (the raw input is
/// checked too: the URL parser silently drops tabs and newlines).
fn redirect_target(url: Option<&str>) -> Result<Option<HeaderValue>, ServeError> {
    let Some(raw) = url else {
        return Ok(None);
    };
    HeaderValue::from_str(raw).map_err(|_| ServeError::InvalidAdminPublicUrl)?;
    let parsed = Url::parse(raw).map_err(|_| ServeError::InvalidAdminPublicUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") || !parsed.has_host() {
        return Err(ServeError::InvalidAdminPublicUrl);
    }
    HeaderValue::from_str(parsed.as_str())
        .map(Some)
        .map_err(|_| ServeError::InvalidAdminPublicUrl)
}

/// The client API router of a split deployment: `GET`/`HEAD /` answers 302 to
/// `admin_public_url`, validated as `Listeners::bind` does. Exposed so the
/// contract tests replay the topology a client sees.
pub fn redirect_client_root(
    client_api: Router,
    admin_public_url: &str,
) -> Result<Router, ServeError> {
    redirect_target(Some(admin_public_url))?
        .map(|location| with_root_redirect(client_api, location))
        .ok_or(ServeError::InvalidAdminPublicUrl)
}

fn with_root_redirect(client_api: Router, location: HeaderValue) -> Router {
    client_api.layer(axum::middleware::from_fn(
        move |req: Request, next: Next| redirect_root(location.clone(), req, next),
    ))
}

async fn redirect_root(location: HeaderValue, req: Request, next: Next) -> Response {
    let is_root = req.uri().path() == "/";
    let is_read = matches!(*req.method(), Method::GET | Method::HEAD);
    if is_root && is_read {
        return (StatusCode::FOUND, [(header::LOCATION, location)]).into_response();
    }
    next.run(req).await
}
