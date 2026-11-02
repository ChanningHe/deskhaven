//! `config check` probes beyond bootstrap validation. All read-only: nothing
//! is created, migrated or generated, and sockets are released at once.

use std::io::{ErrorKind, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, UdpSocket};
use std::time::Duration;

use anyhow::{Context, bail};
use deskhaven_config::bootstrap::{Bootstrap, Loaded};
use deskhaven_config::settings::PinSet;

use crate::database;

/// `config check` gives up on the database sooner than startup does.
const CHECK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Every `[settings]` pin names a known setting and coerces to its type.
pub fn settings_pins(loaded: &Loaded) -> anyhow::Result<()> {
    PinSet::from_bootstrap(&loaded.config.settings)?;
    Ok(())
}

/// The HTTP listen addresses and, when enabled, the signal ports can be
/// bound now (another process may hold them, e.g. a running server).
pub fn listen_ports(loaded: &Loaded) -> anyhow::Result<()> {
    let listen = &loaded.config.listen;
    let admin = listen.admin_addr().map(|addr| ("listen.admin", addr));
    for (key, addr) in std::iter::once(("listen.client_api", listen.client_api)).chain(admin) {
        TcpListener::bind(addr).with_context(|| format!("{key}: cannot bind {addr}"))?;
    }
    if loaded.config.signal.enabled {
        signal_ports(loaded.config.signal.port)?;
    }
    Ok(())
}

/// What the signal server binds on all interfaces: TCP port-1 (NAT test),
/// port (TCP and UDP) and port+2 (WebSocket). Each is probed on IPv4
/// `0.0.0.0` (collides with any IPv4 or dual-stack holder) and on IPv6 `[::]`
/// (collides with any IPv6 holder, e.g. one bound to `[::1]` only). A host
/// without usable IPv6 skips the IPv6 probes. The bootstrap validation
/// already rejected ports without room for port-1 and port+2.
fn signal_ports(port: u16) -> anyhow::Result<()> {
    let tcp = [
        ("signal NAT test (port-1)", port.saturating_sub(1)),
        ("signal.port", port),
        ("signal WebSocket (port+2)", port.saturating_add(2)),
    ];
    for (name, port) in tcp {
        probe(name, "TCP", port, TcpListener::bind)?;
    }
    probe("signal.port", "UDP", port, UdpSocket::bind)
}

/// Binds (and releases) `port` on `0.0.0.0`, then on `[::]`.
fn probe<T>(
    name: &str,
    proto: &str,
    port: u16,
    bind: impl Fn(SocketAddr) -> std::io::Result<T>,
) -> anyhow::Result<()> {
    let v4 = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
    bind(v4).with_context(|| format!("{name}: cannot bind {proto} {v4}"))?;
    let v6 = SocketAddr::from((Ipv6Addr::UNSPECIFIED, port));
    match bind(v6) {
        // Only a taken port counts; any other failure means no usable IPv6 here.
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            Err(err).with_context(|| format!("{name}: cannot bind {proto} {v6}"))
        }
        _ => Ok(()),
    }
}

/// The database is reachable over a read-only connection, and its schema is
/// not newer than this binary. A missing SQLite file is fine (created on
/// first start); pending migrations are reported, not applied.
///
/// PgBouncer in transaction-pool mode rejects the read-only startup parameter
/// (`default_transaction_read_only`) unless it is listed in
/// `ignore_startup_parameters`: this check then fails while `serve` works.
pub fn database(loaded: &Loaded) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    runtime.block_on(database_status(&loaded.config))
}

async fn database_status(config: &Bootstrap) -> anyhow::Result<()> {
    let url = database::url(config)?;
    if database::is_in_memory(&url) {
        return note("database.url is an in-memory database; nothing to check");
    }
    if let Some(file) = deskhaven_db::sqlite_file(&url)
        && !file.exists()
    {
        return note(&format!(
            "database {} does not exist yet; it is created on first start",
            file.display()
        ));
    }
    let mut options = database::options(config, url);
    options.read_only = true;
    options.max_connections = 1;
    options.connect_timeout = CHECK_CONNECT_TIMEOUT;
    let db = database::connect(&options).await?;
    let status = deskhaven_db::migration_status(&db).await;
    db.close().await.context("cannot close the database")?;
    let status = status?;
    if !status.unknown.is_empty() {
        bail!(
            "the database has migrations unknown to this binary ({}); it was upgraded by a newer deskhaven",
            status.unknown.join(", ")
        );
    }
    if !status.pending.is_empty() {
        note(&format!(
            "{} pending migration(s) will be applied on the next start",
            status.pending.len()
        ))?;
    }
    Ok(())
}

fn note(text: &str) -> anyhow::Result<()> {
    writeln!(std::io::stderr().lock(), "note: {text}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    /// A bind that fails only on IPv6 with `kind`.
    fn v6_fails(kind: ErrorKind) -> impl Fn(SocketAddr) -> io::Result<()> {
        move |addr| match addr {
            SocketAddr::V6(_) => Err(io::Error::from(kind)),
            SocketAddr::V4(_) => Ok(()),
        }
    }

    #[test]
    fn probe_reports_a_port_taken_on_ipv6_only() {
        let err = probe("signal.port", "TCP", 21116, v6_fails(ErrorKind::AddrInUse)).unwrap_err();
        assert_eq!(err.to_string(), "signal.port: cannot bind TCP [::]:21116");
    }

    #[test]
    fn probe_skips_ipv6_when_the_host_has_none() {
        for kind in [ErrorKind::AddrNotAvailable, ErrorKind::Unsupported] {
            assert!(
                probe("signal.port", "UDP", 21116, v6_fails(kind)).is_ok(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn probe_reports_ipv4_first() {
        let taken = |_: SocketAddr| -> io::Result<()> { Err(ErrorKind::AddrInUse.into()) };
        let err = probe("signal.port", "UDP", 21116, taken).unwrap_err();
        assert_eq!(
            err.to_string(),
            "signal.port: cannot bind UDP 0.0.0.0:21116"
        );
    }
}
