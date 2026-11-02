// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use hbb_common::{
    anyhow::{Context, Result},
    log, ResultType,
};
use std::{
    net::{IpAddr, SocketAddr},
    time::{Instant, SystemTime},
};

pub fn parse_bind_address(value: &str) -> Result<Option<IpAddr>> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .with_context(|| format!("Invalid bind address: {value}"))
            .map(Some)
    }
}

pub async fn listen_tcp(
    bind_addr: Option<IpAddr>,
    port: u16,
) -> ResultType<hbb_common::tokio::net::TcpListener> {
    if let Some(bind_addr) = bind_addr {
        hbb_common::tcp::new_listener(SocketAddr::new(bind_addr, port), true).await
    } else {
        hbb_common::tcp::listen_any(port).await
    }
}

// deskhaven: `console_addr` / `listen_console` / `accept_or_pending` (listener for the removed
// loopback console) removed.

#[allow(dead_code)]
pub(crate) fn get_expired_time() -> Instant {
    let now = Instant::now();
    now.checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or(now)
}

#[allow(dead_code)]
pub(crate) fn test_if_valid_server(host: &str, name: &str) -> ResultType<SocketAddr> {
    use std::net::ToSocketAddrs;
    let res = if host.contains(':') {
        host.to_socket_addrs()?.next().context("")
    } else {
        format!("{}:{}", host, 0)
            .to_socket_addrs()?
            .next()
            .context("")
    };
    if res.is_err() {
        log::error!("Invalid {} {}: {:?}", name, host, res);
    }
    res
}

// deskhaven: takes a list instead of a comma-separated string.
#[allow(dead_code)]
pub(crate) fn get_servers(s: &[String], tag: &str) -> Vec<String> {
    let servers: Vec<String> = s
        .iter()
        .filter(|x| !x.is_empty() && test_if_valid_server(x, tag).is_ok())
        .cloned()
        .collect();
    log::info!("{}={:?}", tag, servers);
    servers
}

// deskhaven: `arg_name` / `set_arg` / `get_arg_opt` / `get_arg` / `get_arg_or` (arguments read
// from process environment variables) removed; configuration comes from `SignalConfig`.

#[allow(dead_code)]
#[inline]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|x| x.as_secs())
        .unwrap_or_default()
}

// deskhaven: `gen_sk` and its key-file helpers (read/create `id_ed25519` + `.pub` in the CWD)
// removed: the key is injected via `SignalConfig::secret_key` and the key file belongs to
// deskhaven's key material (M0-06). `listen_signal` (own SIGTERM/INT/QUIT handling) removed:
// the embedder stops the server through a `CancellationToken`.

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parses_bind_address() {
        assert_eq!(parse_bind_address("").unwrap(), None);
        assert_eq!(
            parse_bind_address("127.0.0.1").unwrap(),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert_eq!(
            parse_bind_address("::1").unwrap(),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
        assert!(parse_bind_address("not-an-ip").is_err());
    }

    #[hbb_common::tokio::test]
    async fn tcp_listener_uses_bind_address() {
        let bind_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = listen_tcp(Some(bind_addr), 0).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().ip(), bind_addr);
    }

    // deskhaven: replaces the upstream `set_arg` / `get_arg` test; the list form keeps the
    // filtering of empty and unresolvable entries.
    #[test]
    fn servers_drop_empty_and_invalid_entries() {
        let servers = [
            "10.0.0.1:21117".to_owned(),
            String::new(),
            "10.0.0.2".to_owned(),
            "not a host:x".to_owned(),
        ];
        assert_eq!(
            get_servers(&servers, "relay-servers"),
            ["10.0.0.1:21117", "10.0.0.2"]
        );
    }
}
