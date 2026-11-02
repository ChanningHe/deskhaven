//! Tunables for the HTTP stack and its listeners. See the crate docs for how
//! they combine in an internet-facing deployment.

use std::ops::RangeInclusive;
use std::time::Duration;

use axum::http::HeaderValue;

use crate::real_ip::TrustedProxies;

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;
/// nginx's `client_header_timeout` default; long enough that reverse proxies
/// reusing upstream keep-alive connections do not race our idle close (502s).
pub const DEFAULT_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(60);
pub const DEFAULT_WRITE_STALL_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MIN_WRITE_BYTES_PER_SEC: u64 = 1024;
pub const DEFAULT_MAX_CONNECTIONS: usize = 10_000;
/// Generous: whole offices sit behind one NAT address.
pub const DEFAULT_MAX_CONNECTIONS_PER_IP: usize = 256;
/// One host usually owns a whole /64.
pub const DEFAULT_PER_IP_IPV6_PREFIX: u8 = 64;
pub const DEFAULT_MAX_HEADER_BYTES: usize = 64 * 1024;
pub const DEFAULT_MAX_HEADERS: usize = 64;
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// The rest of `RLIMIT_NOFILE` is left to the DB pool, files and the signal server.
pub const DEFAULT_NOFILE_BUDGET_PERCENT: u8 = 50;

const MIN_HEADER_BYTES: usize = 4 * 1024;
const MIN_HEADERS: usize = 16;
const IPV6_PREFIX_RANGE: RangeInclusive<u8> = 32..=128;
const PERCENT_RANGE: RangeInclusive<u8> = 1..=100;

#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Addresses whose `X-Forwarded-For` is honored. Also exempt from the
    /// per-IP connection cap, so a broad range exempts every host inside it:
    /// keep it as narrow as possible.
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
    /// Window over which a blocked response write must average
    /// `min_write_bytes_per_sec`; falling short closes the connection.
    pub write_stall_timeout: Duration,
    /// Minimum response throughput for a client whose writes block; 0 = any
    /// progress per window is enough.
    pub min_write_bytes_per_sec: u64,
    /// Open connections per listener; further connections wait in the backlog.
    /// Clamped to the listener's `RLIMIT_NOFILE` share at serve time.
    pub max_connections: usize,
    /// Open connections per client (IPv4 address or IPv6 prefix) and listener;
    /// excess connections are closed at once. Effective cap:
    /// `min(this, max(16, effective max_connections / 64))`. Trusted proxies
    /// are exempt.
    pub max_connections_per_ip: usize,
    /// IPv6 clients are grouped by this prefix for the per-IP cap (32..=128).
    pub per_ip_ipv6_prefix: u8,
    /// Max size of a request head (request line + headers, >= 4 KiB); -> 431.
    pub max_header_bytes: usize,
    /// Max number of request headers (>= 16); exceeded -> 431.
    pub max_headers: usize,
    /// After shutdown is requested, in-flight requests get this long before their
    /// connections are dropped.
    pub drain_timeout: Duration,
    /// Share of the soft `RLIMIT_NOFILE` HTTP connections may use (1..=100).
    pub nofile_budget_percent: u8,
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
            min_write_bytes_per_sec: DEFAULT_MIN_WRITE_BYTES_PER_SEC,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_connections_per_ip: DEFAULT_MAX_CONNECTIONS_PER_IP,
            per_ip_ipv6_prefix: DEFAULT_PER_IP_IPV6_PREFIX,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            max_headers: DEFAULT_MAX_HEADERS,
            drain_timeout: DEFAULT_DRAIN_TIMEOUT,
            nofile_budget_percent: DEFAULT_NOFILE_BUDGET_PERCENT,
            content_security_policy: None,
            strict_transport_security: None,
        }
    }
}

/// A setting outside its allowed range.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid http setting {field}: must be {expected}")]
pub struct ConfigError {
    pub field: &'static str,
    pub expected: &'static str,
}

impl HttpConfig {
    /// Checked by [`crate::Listeners::serve`] before anything is served.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let checks = [
            (
                self.max_header_bytes >= MIN_HEADER_BYTES,
                "max_header_bytes",
                ">= 4096",
            ),
            (self.max_headers >= MIN_HEADERS, "max_headers", ">= 16"),
            (
                IPV6_PREFIX_RANGE.contains(&self.per_ip_ipv6_prefix),
                "per_ip_ipv6_prefix",
                "in 32..=128",
            ),
            (self.max_connections >= 1, "max_connections", ">= 1"),
            (
                self.max_connections_per_ip >= 1,
                "max_connections_per_ip",
                ">= 1",
            ),
            (self.body_limit >= 1, "body_limit", ">= 1"),
            (!self.request_timeout.is_zero(), "request_timeout", "> 0"),
            (
                !self.header_read_timeout.is_zero(),
                "header_read_timeout",
                "> 0",
            ),
            (
                !self.write_stall_timeout.is_zero(),
                "write_stall_timeout",
                "> 0",
            ),
            (
                PERCENT_RANGE.contains(&self.nofile_budget_percent),
                "nofile_budget_percent",
                "in 1..=100",
            ),
        ];
        checks
            .into_iter()
            .find(|(is_valid, ..)| !is_valid)
            .map_or(Ok(()), |(_, field, expected)| {
                Err(ConfigError { field, expected })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(cfg: HttpConfig) -> &'static str {
        cfg.validate().unwrap_err().field
    }

    #[test]
    fn defaults_are_valid() {
        assert_eq!(HttpConfig::default().validate(), Ok(()));
    }

    /// Review round 3 LOW: limits are validated, not silently turned into 431s.
    #[test]
    fn out_of_range_limits_are_rejected() {
        let base = HttpConfig::default;
        let cases = [
            (
                HttpConfig {
                    max_header_bytes: 4095,
                    ..base()
                },
                "max_header_bytes",
            ),
            (
                HttpConfig {
                    max_headers: 15,
                    ..base()
                },
                "max_headers",
            ),
            (
                HttpConfig {
                    per_ip_ipv6_prefix: 31,
                    ..base()
                },
                "per_ip_ipv6_prefix",
            ),
            (
                HttpConfig {
                    per_ip_ipv6_prefix: 129,
                    ..base()
                },
                "per_ip_ipv6_prefix",
            ),
            (
                HttpConfig {
                    max_connections: 0,
                    ..base()
                },
                "max_connections",
            ),
            (
                HttpConfig {
                    max_connections_per_ip: 0,
                    ..base()
                },
                "max_connections_per_ip",
            ),
            (
                HttpConfig {
                    body_limit: 0,
                    ..base()
                },
                "body_limit",
            ),
            (
                HttpConfig {
                    request_timeout: Duration::ZERO,
                    ..base()
                },
                "request_timeout",
            ),
            (
                HttpConfig {
                    header_read_timeout: Duration::ZERO,
                    ..base()
                },
                "header_read_timeout",
            ),
            (
                HttpConfig {
                    write_stall_timeout: Duration::ZERO,
                    ..base()
                },
                "write_stall_timeout",
            ),
            (
                HttpConfig {
                    nofile_budget_percent: 0,
                    ..base()
                },
                "nofile_budget_percent",
            ),
            (
                HttpConfig {
                    nofile_budget_percent: 101,
                    ..base()
                },
                "nofile_budget_percent",
            ),
        ];
        for (cfg, field) in cases {
            assert_eq!(rejected(cfg), field);
        }
    }

    #[test]
    fn boundary_values_are_accepted() {
        let cfg = HttpConfig {
            max_header_bytes: 4096,
            max_headers: 16,
            per_ip_ipv6_prefix: 32,
            nofile_budget_percent: 100,
            ..HttpConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
        let cfg = HttpConfig {
            per_ip_ipv6_prefix: 128,
            nofile_budget_percent: 1,
            ..HttpConfig::default()
        };
        assert_eq!(cfg.validate(), Ok(()));
    }
}
