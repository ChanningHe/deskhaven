//! Tunables for the HTTP stack and its listeners. See the crate docs for how
//! they combine in an internet-facing deployment.

use std::time::Duration;

use axum::http::HeaderValue;

use crate::real_ip::TrustedProxies;

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;
/// nginx's `client_header_timeout` default; long enough that reverse proxies
/// reusing upstream keep-alive connections do not race our idle close (502s).
pub const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_WRITE_STALL_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MAX_CONNECTIONS: usize = 10_000;
/// Generous: whole offices sit behind one NAT address.
pub const DEFAULT_MAX_CONNECTIONS_PER_IP: usize = 256;
pub const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_HEADERS: usize = 64;
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub trusted_proxies: TrustedProxies,
    /// Whole-request deadline (until response headers, including reading the
    /// body the handler consumes); exceeded -> 503.
    pub request_timeout: Duration,
    /// Max request body in bytes; exceeded -> 413. Replaces axum's 2 MiB default.
    pub body_limit: usize,
    /// Time a client gets to send a complete request head (default 60s). hyper
    /// runs the same timer while a keep-alive connection waits for its next
    /// request, so this is also the idle keep-alive timeout; reverse proxies
    /// should reuse upstream connections for less than this.
    pub header_read_timeout: Duration,
    /// A response write with no progress for this long closes the connection
    /// (a client that stops reading cannot pin a slot).
    pub write_stall_timeout: Duration,
    /// Open connections per listener; further connections wait in the backlog.
    /// Clamped to the `RLIMIT_NOFILE` budget at serve time.
    pub max_connections: usize,
    /// Open connections per client address and listener; excess connections are
    /// closed at once. Trusted proxies are exempt.
    pub max_connections_per_ip: usize,
    /// Max size of a request head (request line + headers); exceeded -> 431.
    pub max_header_bytes: usize,
    /// Max number of request headers; exceeded -> 431.
    pub max_headers: usize,
    /// After shutdown is requested, in-flight requests get this long before their
    /// connections are dropped.
    pub drain_timeout: Duration,
    /// Hook for the web crate: `Content-Security-Policy` for responses that did
    /// not set one. In single-listener mode the API and the SPA share it.
    pub content_security_policy: Option<HeaderValue>,
    /// `Strict-Transport-Security` value; set it only when `public_url` is https.
    pub strict_transport_security: Option<HeaderValue>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            trusted_proxies: TrustedProxies::default(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            body_limit: DEFAULT_BODY_LIMIT,
            header_read_timeout: DEFAULT_HEADER_READ_TIMEOUT,
            write_stall_timeout: DEFAULT_WRITE_STALL_TIMEOUT,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_connections_per_ip: DEFAULT_MAX_CONNECTIONS_PER_IP,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            max_headers: DEFAULT_MAX_HEADERS,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            content_security_policy: None,
            strict_transport_security: None,
        }
    }
}
