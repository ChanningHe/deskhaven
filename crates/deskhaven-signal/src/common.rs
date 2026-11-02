// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use hbb_common::{
    ResultType,
    futures::future::join_all,
    tokio::{net::lookup_host, time::timeout},
};
use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
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

/// Per-entry bound for relay-server lookups.
pub(crate) const RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

// deskhaven: async replacement for upstream `test_if_valid_server` / `get_servers`, whose
// blocking `ToSocketAddrs` lookups ran on the server loop. Same acceptance rule: `host:port`
// as given, a bare host with port 0; empty entries are dropped silently. Entries resolve
// concurrently, each bounded by `limit`; a lookup that times out counts as rejected. Returns
// `(accepted, rejected)` in input order; logging is left to the caller.
pub(crate) async fn resolve_servers(
    servers: &[String],
    limit: Duration,
) -> (Vec<String>, Vec<String>) {
    resolve_servers_with(servers, limit, dns_lookup).await
}

async fn dns_lookup(target: String) -> bool {
    lookup_host(target)
        .await
        .is_ok_and(|mut addrs| addrs.next().is_some())
}

/// `resolve_servers` with an injectable lookup (tests).
async fn resolve_servers_with<F, Fut>(
    servers: &[String],
    limit: Duration,
    lookup: F,
) -> (Vec<String>, Vec<String>)
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = bool>,
{
    let hosts: Vec<&String> = servers.iter().filter(|x| !x.is_empty()).collect();
    let results = join_all(hosts.iter().map(|host| {
        let target = if host.contains(':') {
            (*host).clone()
        } else {
            format!("{host}:0")
        };
        let lookup = lookup(target);
        async move { timeout(limit, lookup).await.unwrap_or(false) }
    }))
    .await;
    let (accepted, rejected): (Vec<_>, Vec<_>) =
        hosts.into_iter().zip(results).partition(|(_, ok)| *ok);
    let names = |list: Vec<(&String, bool)>| list.into_iter().map(|(h, _)| h.clone()).collect();
    (names(accepted), names(rejected))
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
        let (accepted, rejected) = resolve_servers(&servers, RESOLVE_TIMEOUT).await;
        assert_eq!(accepted, ["10.0.0.1:21117", "10.0.0.2"]);
        assert_eq!(rejected, ["not a host:x"]);
    }

    // A lookup that does not finish in time counts as rejected, and lookups run concurrently:
    // three hanging entries cost about one timeout, not three.
    #[hbb_common::tokio::test]
    async fn resolve_servers_times_out_hanging_lookups_concurrently() {
        const LIMIT: Duration = Duration::from_millis(300);
        let servers: Vec<String> = ["hang-1", "hang-2:1", "hang-3", "ok:1"]
            .map(str::to_owned)
            .to_vec();
        let lookup = |target: String| async move {
            if target.starts_with("hang") {
                std::future::pending::<bool>().await
            } else {
                true
            }
        };
        let started = Instant::now();

        let (accepted, rejected) = resolve_servers_with(&servers, LIMIT, lookup).await;

        assert_eq!(accepted, ["ok:1"]);
        assert_eq!(rejected, ["hang-1", "hang-2:1", "hang-3"]);
        assert!(
            started.elapsed() < LIMIT * 2,
            "concurrent: {:?}",
            started.elapsed()
        );
    }
}
