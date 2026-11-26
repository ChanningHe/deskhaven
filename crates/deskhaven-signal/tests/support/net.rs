use hbb_common::tokio::time::Duration;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Mutex, Once, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
};

pub const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
pub const STEP: Duration = Duration::from_secs(5);
pub const STEP_MS: u64 = 5_000;
pub const QUIET: Duration = Duration::from_millis(500);
/// Mirrors the server's `REG_TIMEOUT`.
pub const REG_TIMEOUT: Duration = Duration::from_millis(30_000);

const CLIENT_NET: [u8; 2] = [127, 77];
const HOSTS_PER_OCTET: u32 = 250;
const FIRST_HOST: u32 = 2;
static NEXT_CLIENT: AtomicU32 = AtomicU32::new(0);
static LOOPBACK_PROBE: Once = Once::new();

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

/// So per-IP server state never leaks between tests.
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

pub fn ports_free(port: u16) -> bool {
    std::net::TcpListener::bind(addr(port - 1)).is_ok()
        && std::net::TcpListener::bind(addr(port)).is_ok()
        && std::net::TcpListener::bind(addr(port + 2)).is_ok()
        && std::net::UdpSocket::bind(addr(port)).is_ok()
}

static RESERVED_PORTS: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// Reserves the whole span in-process, so two parallel tests of one binary never race for
/// ports both probed free; losing a race to another process is the caller's to cope with.
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
