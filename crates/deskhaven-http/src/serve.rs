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

use crate::middleware::{HttpConfig, apply_middleware};

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
    pub admin_public_url: Option<String>,
}

/// Bare routers (no middleware): [`Listeners::serve`] applies the stack.
///
/// Single listener: requests not matched by `client_api` fall through to
/// `admin` (any fallback on `client_api` is replaced). Split: each router is
/// served on its own listener.
pub struct Apps {
    pub client_api: Router,
    pub admin: Router,
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
    #[error("listen.admin_public_url is not a valid Location header value")]
    InvalidAdminPublicUrl,
    #[error("{name} listener failed")]
    Serve {
        name: &'static str,
        #[source]
        source: io::Error,
    },
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
        let client_api = bind_one(CLIENT_API, cfg.client_api).await?;
        let admin = match cfg.admin {
            None => None,
            Some(addr) => Some(AdminListener {
                redirect: redirect_target(cfg.admin_public_url.as_deref())?,
                bound: bind_one(ADMIN, addr).await?,
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

    /// Serve until `shutdown` is cancelled, then drain in-flight requests.
    /// If one listener stops, the other is shut down too.
    pub async fn serve(
        self,
        apps: Apps,
        cfg: &HttpConfig,
        shutdown: CancellationToken,
    ) -> Result<(), ServeError> {
        let shutdown = shutdown.child_token();
        let Some(admin) = self.admin else {
            let router = apply_middleware(apps.client_api.fallback_service(apps.admin), cfg);
            return serve_one(CLIENT_API, self.client_api.listener, router, shutdown).await;
        };
        let client_router = match admin.redirect {
            Some(location) => apps.client_api.layer(axum::middleware::from_fn(
                move |req: Request, next: Next| redirect_root(location.clone(), req, next),
            )),
            None => apps.client_api,
        };
        let (client, admin) = tokio::join!(
            serve_one(
                CLIENT_API,
                self.client_api.listener,
                apply_middleware(client_router, cfg),
                shutdown.clone(),
            ),
            serve_one(
                ADMIN,
                admin.bound.listener,
                apply_middleware(apps.admin, cfg),
                shutdown
            ),
        );
        client.and(admin)
    }
}

fn redirect_target(url: Option<&str>) -> Result<Option<HeaderValue>, ServeError> {
    url.filter(|u| !u.is_empty())
        .map(|u| HeaderValue::from_str(u).map_err(|_| ServeError::InvalidAdminPublicUrl))
        .transpose()
}

async fn redirect_root(location: HeaderValue, req: Request, next: Next) -> Response {
    let is_root = req.uri().path() == "/";
    let is_read = matches!(*req.method(), Method::GET | Method::HEAD);
    if is_root && is_read {
        return (StatusCode::FOUND, [(header::LOCATION, location)]).into_response();
    }
    next.run(req).await
}

async fn serve_one(
    name: &'static str,
    listener: TcpListener,
    router: Router,
    shutdown: CancellationToken,
) -> Result<(), ServeError> {
    let service = router.into_make_service_with_connect_info::<SocketAddr>();
    let result = axum::serve(listener, service)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned())
        .await;
    // Whatever ended this listener, stop its sibling as well.
    shutdown.cancel();
    result.map_err(|source| ServeError::Serve { name, source })
}
