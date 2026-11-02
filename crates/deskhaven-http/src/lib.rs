//! Shared HTTP plumbing for the client and admin APIs: real client IP,
//! middleware stack, stack-error rendering, health routes and listeners.
//!
//! # Deployment
//!
//! Internet-facing deployments should sit behind a reverse proxy (TLS, request
//! buffering, connection management) with its address in `trusted_proxies`.
//! The built-in limits keep a directly exposed server alive but are not a
//! substitute. Knobs ([`HttpConfig`], checked by [`HttpConfig::validate`]):
//!
//! | knob | default | bounds |
//! |---|---|---|
//! | `header_read_timeout` | 60s | sending the request head; idle keep-alive |
//! | `request_timeout` | 30s | handler incl. reading the body (-> 503) |
//! | `write_stall_timeout` + `min_write_bytes_per_sec` | 30s, 1 KiB/s | clients reading the response too slowly |
//! | `body_limit` | 2 MiB | request body (-> 413) |
//! | `max_header_bytes` / `max_headers` | 64 KiB / 64 | request head (-> 431) |
//! | `max_connections` | 10 000 | per listener; HTTP gets `nofile_budget_percent` (50%) of `RLIMIT_NOFILE`, split 3:1 client_api:admin |
//! | `max_connections_per_ip` | 256 | per untrusted IPv4 address / IPv6 `/per_ip_ipv6_prefix` (64), capped at `max(16, max_connections / 64)` |
//! | `drain_timeout` | 10s | shutdown |
//!
//! Trusted proxies are exempt from the per-IP cap, so keep `trusted_proxies` as
//! narrow as possible: a broad range exempts every host inside it.
//!
//! Call [`raise_nofile_limit`] once at startup. The proxy must overwrite, not
//! forward, the client's `X-Forwarded-For` and `X-Request-Id`, e.g. nginx:
//!
//! ```nginx
//! proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
//! proxy_set_header X-Request-Id    $request_id;
//! ```
//!
//! (`$proxy_add_x_forwarded_for` appends the peer it saw, which is what the
//! right-to-left walk in [`TrustedProxies::resolve`] expects.)

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
pub use real_ip::{ClientIp, ClientIpRejection, InternalPeer, TrustedProxies, X_FORWARDED_FOR};
pub use request_id::X_REQUEST_ID;
pub use serve::{Apps, ListenConfig, Listeners, ServeError};
pub use stack_error::{ErrorBodies, ErrorRenderer, StackError, plain_text};
