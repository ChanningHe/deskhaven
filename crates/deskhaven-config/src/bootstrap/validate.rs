//! Semantic checks that serde types cannot express.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use tracing_subscriber::EnvFilter;

use super::error::Warning;
use super::schema::Bootstrap;

const URL_SCHEMES: [&str; 2] = ["http", "https"];
const DATABASE_SCHEMES: [&str; 3] = ["sqlite", "postgres", "postgresql"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proto {
    Tcp,
    Udp,
}

/// One socket the process will bind.
struct Binding {
    name: &'static str,
    proto: Proto,
    addr: SocketAddr,
}

/// Returns all problems found (empty = valid) plus non-fatal warnings.
pub(crate) fn validate(cfg: &Bootstrap) -> (Vec<String>, Vec<Warning>) {
    let mut problems = Vec::new();
    problems.extend(check_url("public_url", &cfg.public_url));
    problems.extend(check_url(
        "listen.admin_public_url",
        &cfg.listen.admin_public_url,
    ));
    problems.extend(check_log_level(&cfg.log.level));
    problems.extend(check_database(cfg));
    problems.extend(check_signal_port(cfg));
    problems.extend(port_conflicts(&bindings(cfg)));
    if cfg.data_dir.as_os_str().is_empty() {
        problems.push("data_dir: must not be empty".to_owned());
    }

    let mut warnings = Vec::new();
    if cfg.listen.admin_addr().is_some() && cfg.listen.admin_public_url.is_empty() {
        warnings.push(Warning::AdminRedirectUnset);
    }
    (problems, warnings)
}

/// Never echoes the value: a malformed URL may still carry credentials.
fn check_url(key: &str, value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    let url = match url::Url::parse(value) {
        Ok(url) => url,
        Err(e) => return Some(format!("{key}: invalid URL: {e}")),
    };
    let reason = if !URL_SCHEMES.contains(&url.scheme()) {
        "scheme must be http or https"
    } else if url.host().is_none() {
        "a host is required"
    } else if !url.username().is_empty() || url.password().is_some() {
        "credentials are not allowed"
    } else if url.query().is_some() || url.fragment().is_some() {
        "query and fragment are not allowed"
    } else {
        return None;
    };
    Some(format!("{key}: {reason}"))
}

fn check_log_level(level: &str) -> Option<String> {
    EnvFilter::builder()
        .parse(level)
        .err()
        .map(|e| format!("log.level: invalid filter {level:?}: {e}"))
}

/// Never echoes the URL: it may carry credentials.
fn check_database(cfg: &Bootstrap) -> Vec<String> {
    let mut problems = Vec::new();
    let url = cfg.database.url.expose();
    if !url.is_empty() {
        let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
        if !scheme.is_some_and(|s| DATABASE_SCHEMES.contains(&s.as_str())) {
            problems.push("database.url: scheme must be sqlite or postgres".to_owned());
        }
    }
    if cfg.database.max_connections == 0 {
        problems.push("database.max_connections: must be at least 1".to_owned());
    }
    problems
}

/// hbbs also binds port-1 (NAT test) and port+2 (WebSocket).
fn check_signal_port(cfg: &Bootstrap) -> Option<String> {
    let port = cfg.signal.port;
    let in_range = port.checked_sub(1).is_some_and(|p| p > 0) && port.checked_add(2).is_some();
    (cfg.signal.enabled && !in_range)
        .then(|| format!("signal.port: {port} leaves no room for port-1 and port+2"))
}

fn bindings(cfg: &Bootstrap) -> Vec<Binding> {
    let bind = |name, proto, addr| Binding { name, proto, addr };
    // Out-of-range neighbours become port 0 (skipped); check_signal_port reports them.
    let any = |port: Option<u16>| SocketAddr::from((Ipv4Addr::UNSPECIFIED, port.unwrap_or(0)));

    let mut out = vec![bind("listen.client_api", Proto::Tcp, cfg.listen.client_api)];
    out.extend(
        cfg.listen
            .admin_addr()
            .map(|addr| bind("listen.admin", Proto::Tcp, addr)),
    );
    if cfg.signal.enabled {
        let port = cfg.signal.port;
        out.extend([
            bind("signal.port", Proto::Tcp, any(Some(port))),
            bind("signal.port", Proto::Udp, any(Some(port))),
            bind(
                "signal NAT test (port-1)",
                Proto::Tcp,
                any(port.checked_sub(1)),
            ),
            bind(
                "signal WebSocket (port+2)",
                Proto::Tcp,
                any(port.checked_add(2)),
            ),
        ]);
    }
    out
}

/// Whether binding both addresses on one port can collide. `[::]` may be
/// dual-stack, so it overlaps everything; `0.0.0.0` overlaps all IPv4.
fn ips_overlap(a: IpAddr, b: IpAddr) -> bool {
    let v6_any = |ip: IpAddr| ip.is_ipv6() && ip.is_unspecified();
    let v4_any_covers =
        |any: IpAddr, other: IpAddr| any.is_ipv4() && any.is_unspecified() && other.is_ipv4();
    a == b || v6_any(a) || v6_any(b) || v4_any_covers(a, b) || v4_any_covers(b, a)
}

/// Port 0 (OS-assigned) never conflicts.
fn port_conflicts(bindings: &[Binding]) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, a) in bindings.iter().enumerate() {
        for b in bindings.iter().skip(i + 1) {
            let same_port = a.addr.port() == b.addr.port() && a.addr.port() != 0;
            let overlap = ips_overlap(a.addr.ip(), b.addr.ip());
            if a.proto == b.proto && same_port && overlap {
                problems.push(format!(
                    "{} ({}) conflicts with {} ({}) on port {}",
                    a.name,
                    a.addr,
                    b.name,
                    b.addr,
                    a.addr.port()
                ));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: &str, b: &str) -> bool {
        let (a, b): (IpAddr, IpAddr) = (a.parse().unwrap(), b.parse().unwrap());
        assert_eq!(ips_overlap(a, b), ips_overlap(b, a), "asymmetric");
        ips_overlap(a, b)
    }

    #[test]
    fn v4_unspecified_overlaps_v4_and_v6_unspecified_only() {
        assert!(overlap("0.0.0.0", "127.0.0.1"));
        assert!(overlap("0.0.0.0", "::"));
        assert!(!overlap("0.0.0.0", "::1"));
    }

    #[test]
    fn v6_unspecified_overlaps_everything() {
        assert!(overlap("::", "::1"));
        assert!(overlap("::", "10.0.0.1"));
    }

    #[test]
    fn specific_addresses_overlap_only_when_equal() {
        assert!(!overlap("127.0.0.1", "10.0.0.1"));
        assert!(overlap("127.0.0.1", "127.0.0.1"));
    }

    #[test]
    fn accepts_common_log_filters() {
        assert_eq!(check_log_level("info"), None);
        assert_eq!(check_log_level("warn,deskhaven=debug"), None);
    }
}
