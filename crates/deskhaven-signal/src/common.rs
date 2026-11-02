// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use hbb_common::{ResultType, log, tokio::net::lookup_host};
use std::{
    net::{IpAddr, SocketAddr},
    time::Instant,
};

// deskhaven: `parse_bind_address` (only used by upstream `main.rs`) removed; the module is
// crate-private since M1-03.

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

// deskhaven: async replacement for upstream `test_if_valid_server` / `get_servers`, whose
// blocking `ToSocketAddrs` lookups ran on the server loop. Same acceptance rule: `host:port`
// as given, a bare host with port 0; empty entries are dropped silently. Returns
// `(accepted, rejected)` in input order.
pub(crate) async fn resolve_servers(servers: &[String], tag: &str) -> (Vec<String>, Vec<String>) {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for host in servers.iter().filter(|x| !x.is_empty()) {
        if resolves(host).await {
            accepted.push(host.clone());
        } else {
            log::error!("Invalid {} {}", tag, host);
            rejected.push(host.clone());
        }
    }
    log::info!("{}={:?}", tag, accepted);
    (accepted, rejected)
}

async fn resolves(host: &str) -> bool {
    let target = if host.contains(':') {
        host.to_owned()
    } else {
        format!("{host}:0")
    };
    lookup_host(target)
        .await
        .is_ok_and(|mut addrs| addrs.next().is_some())
}

// deskhaven: `arg_name` / `set_arg` / `get_arg_opt` / `get_arg` / `get_arg_or` (arguments read
// from process environment variables) removed; configuration comes from `SignalConfig`.
// `now` (unused) removed.

// deskhaven: `gen_sk` and its key-file helpers (read/create `id_ed25519` + `.pub` in the CWD)
// removed: the key is injected via `SignalConfig::secret_key` and the key file belongs to
// deskhaven's key material (M0-06). `listen_signal` (own SIGTERM/INT/QUIT handling) removed:
// the embedder stops the server through a `CancellationToken`.

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio;
    use std::net::Ipv4Addr;

    #[hbb_common::tokio::test]
    async fn tcp_listener_uses_bind_address() {
        let bind_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = listen_tcp(Some(bind_addr), 0).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().ip(), bind_addr);
    }

    // deskhaven: replaces the upstream `set_arg` / `get_arg` test. No DNS needed: literals
    // resolve locally and the rejected entry has an invalid port.
    #[hbb_common::tokio::test]
    async fn resolve_servers_splits_accepted_and_rejected() {
        let servers = [
            "10.0.0.1:21117".to_owned(),
            String::new(),
            "10.0.0.2".to_owned(),
            "not a host:x".to_owned(),
        ];
        let (accepted, rejected) = resolve_servers(&servers, "relay-servers").await;
        assert_eq!(accepted, ["10.0.0.1:21117", "10.0.0.2"]);
        assert_eq!(rejected, ["not a host:x"]);
    }
}
