//! Addresses, ports and timing constants.

use hbb_common::tokio::time::Duration;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Mutex, Once, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
};

/// Where every test server listens.
pub const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// Upper bound for anything that must happen (connect, an answer, a shutdown).
pub const STEP: Duration = Duration::from_secs(5);
pub const STEP_MS: u64 = 5_000;
/// How long to wait for a message that must NOT arrive.
pub const QUIET: Duration = Duration::from_millis(500);
/// Upstream `REG_TIMEOUT` (rendezvous_server.rs): a peer silent this long is offline.
pub const REG_TIMEOUT: Duration = Duration::from_millis(30_000);

/// Client addresses: 127.77.0.2 upwards, one per call.
const CLIENT_NET: [u8; 2] = [127, 77];
const HOSTS_PER_OCTET: u32 = 250;
const FIRST_HOST: u32 = 2;
static NEXT_CLIENT: AtomicU32 = AtomicU32::new(0);
static LOOPBACK_PROBE: Once = Once::new();

/// Fails fast, with the reason, where 127.77.0.0/16 is not routed to the loopback interface.
fn ensure_loopback_net() {
    LOOPBACK_PROBE.call_once(|| {
        let ip = Ipv4Addr::new(CLIENT_NET[0], CLIENT_NET[1], 0, 1);
        if let Err(err) = std::net::UdpSocket::bind(SocketAddr::new(ip.into(), 0)) {
            panic!(
                "cannot bind {ip} ({err}): the signal protocol tests require Linux 127.0.0.0/8 \
                 loopback routing (on macOS add lo0 aliases for 127.77.0.0/16)"
            );
        }
    });
}

/// A loopback source address no other client in this process has used, so per-IP server
/// state (`IP_BLOCKER`) never leaks between tests.
pub fn next_client_ip() -> IpAddr {
    ensure_loopback_net();
    let n = NEXT_CLIENT.fetch_add(1, Ordering::Relaxed);
    let hi = u8::try_from(n / HOSTS_PER_OCTET).unwrap();
    let lo = u8::try_from(n % HOSTS_PER_OCTET + FIRST_HOST).unwrap();
    IpAddr::V4(Ipv4Addr::new(CLIENT_NET[0], CLIENT_NET[1], hi, lo))
}

pub fn addr(port: u16) -> SocketAddr {
    SocketAddr::new(LOCALHOST, port)
}

/// The server derives two more TCP ports from its main port: NAT test on port-1 and
/// websocket on port+2.
pub fn ports_free(port: u16) -> bool {
    std::net::TcpListener::bind(addr(port - 1)).is_ok()
        && std::net::TcpListener::bind(addr(port)).is_ok()
        && std::net::TcpListener::bind(addr(port + 2)).is_ok()
        && std::net::UdpSocket::bind(addr(port)).is_ok()
}

/// Ports (`port - 1 ..= port + 2`) handed out by [`free_base_port`] in this process; never
/// handed out again.
static RESERVED_PORTS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// A main port whose derived ports are all free right now and were never handed out to
/// another test of this process (the caller still has to cope with losing a race for them
/// to another process).
///
/// The probe alone is not enough: two tests that probe the same triple before either server
/// binds it race for it (since M1-08 the loser fails with a Bind error and retries; before,
/// SO_REUSEPORT let both bind and the second stole the first's traffic). Reserving the whole
/// span avoids that race between the parallel tests of one binary. The reservation is in-process only: `cargo test` runs the
/// test binaries one after another, so that suffices; a runner that runs binaries in parallel
/// (e.g. nextest) would need a cross-process scheme (lock files, per-binary port ranges).
pub fn free_base_port() -> u16 {
    for _ in 0..100 {
        let port = std::net::TcpListener::bind(addr(0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if !(3..=65533).contains(&port) {
            continue;
        }
        let span = port - 1..=port + 2;
        let mut reserved = RESERVED_PORTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if span.clone().any(|p| reserved.contains(&p)) || !ports_free(port) {
            continue;
        }
        reserved.extend(span);
        return port;
    }
    panic!("no free port triple found");
}
