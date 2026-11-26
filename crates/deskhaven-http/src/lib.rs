//! Internet-facing deployments should sit behind a reverse proxy listed in `trusted_proxies`;
//! the built-in limits ([`HttpConfig`]) keep a directly exposed server alive but are no
//! substitute. The proxy must overwrite, not forward, the client's `X-Forwarded-For` and
//! `X-Request-Id`, e.g. nginx:
//!
//! ```nginx
//! proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
//! proxy_set_header X-Request-Id    $request_id;
//! ```

mod config;
mod conn;
mod error;
mod health;
mod io;
mod middleware;
mod nofile;
mod paths;
mod per_ip;
mod real_ip;
mod request_id;
mod serve;
mod stack_error;

pub use config::{
    ConfigError, DEFAULT_BODY_LIMIT, DEFAULT_DRAIN_TIMEOUT, DEFAULT_HEADER_READ_TIMEOUT,
    DEFAULT_MAX_CONNECTIONS, DEFAULT_MAX_CONNECTIONS_PER_IP, DEFAULT_MAX_HEADER_BYTES,
    DEFAULT_MAX_HEADERS, DEFAULT_MIN_WRITE_BYTES_PER_SEC, DEFAULT_NOFILE_BUDGET_PERCENT,
    DEFAULT_PER_IP_IPV6_PREFIX, DEFAULT_REQUEST_TIMEOUT, DEFAULT_WRITE_STALL_TIMEOUT, HttpConfig,
};
pub use error::error_chain;
pub use health::{NotReady, ReadinessCheck, health_routes};
pub use middleware::{INTERNAL_ERROR_BODY, apply_middleware};
pub use nofile::{NOFILE_TARGET_WHEN_UNLIMITED, NofileInfo, raise_nofile_limit};
pub use paths::{ADMIN_API_PREFIX, CLIENT_API_PREFIX};
pub use real_ip::{
    ClientIp, ClientIpRejection, InternalPeer, TrustedProxies, X_FORWARDED_FOR, socket_peer,
};
pub use request_id::X_REQUEST_ID;
pub use serve::{
    Apps, ListenConfig, Listeners, ServeError, redirect_client_root, single_listener_router,
};
pub use stack_error::{ErrorBodies, ErrorRenderer, StackError, plain_text};
