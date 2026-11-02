//! Per-connection addresses for websocket clients whose IP comes from a reverse proxy's
//! `X-Real-IP` / `X-Forwarded-For` header (deskhaven code, not vendored; M1-08).
//!
//! The server keys connection state by client address (`tcp_punch`, the address a target
//! echoes back to reach the controller). Upstream gave every such client `<header ip>:0`, so
//! all clients behind one proxy IP shared one key and overwrote each other's state. Each
//! connection now holds `<header ip>:<port>` with a port no other live connection claiming that
//! IP holds; the IP stays what the header says (LAN checks, relay choice, logs).

use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

#[derive(Default)]
struct Claims {
    /// The last port handed out; the next claim tries the one after it.
    last: u16,
    live: HashSet<SocketAddr>,
}

/// Hands out addresses unique among the live connections of one server.
#[derive(Clone, Default)]
pub(crate) struct ClaimedAddrs(Arc<Mutex<Claims>>);

/// An address held by one connection, released when dropped.
pub(crate) struct ClaimedAddr {
    addr: SocketAddr,
    owner: ClaimedAddrs,
}

impl ClaimedAddrs {
    /// `ip` with a port (1..=65535, in turn) that no live claim of `ip` holds; `None` if all
    /// 65535 are held.
    pub(crate) fn claim(&self, ip: IpAddr) -> Option<ClaimedAddr> {
        let mut claims = self.lock();
        for _ in 0..u16::MAX {
            claims.last = claims.last.checked_add(1).unwrap_or(1);
            let addr = SocketAddr::new(ip, claims.last);
            if claims.live.insert(addr) {
                return Some(ClaimedAddr {
                    addr,
                    owner: self.clone(),
                });
            }
        }
        None
    }

    fn lock(&self) -> MutexGuard<'_, Claims> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl ClaimedAddr {
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for ClaimedAddr {
    fn drop(&mut self) {
        self.owner.lock().live.remove(&self.addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
    const OTHER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 2));

    #[test]
    fn live_claims_of_one_ip_get_distinct_non_zero_ports() {
        // Arrange
        let addrs = ClaimedAddrs::default();

        // Act
        let claims: Vec<ClaimedAddr> = (0..100).filter_map(|_| addrs.claim(IP)).collect();

        // Assert
        let ports: HashSet<u16> = claims.iter().map(|c| c.addr().port()).collect();
        assert_eq!(ports.len(), 100);
        assert!(!ports.contains(&0));
        assert!(claims.iter().all(|c| c.addr().ip() == IP));
    }

    #[test]
    fn a_dropped_claim_frees_its_port_and_every_port_is_used_before_none() {
        // Arrange
        let addrs = ClaimedAddrs::default();
        let mut claims: Vec<ClaimedAddr> = (0..u16::MAX).filter_map(|_| addrs.claim(IP)).collect();
        assert_eq!(claims.len(), usize::from(u16::MAX));

        // Act
        let exhausted = addrs.claim(IP).is_none();
        let other_ip = addrs.claim(OTHER_IP).map(|c| c.addr());
        let freed = claims.swap_remove(1234).addr();
        let reused = addrs.claim(IP).map(|c| c.addr());

        // Assert
        assert!(exhausted, "all 65535 ports of the IP are held");
        assert_eq!(other_ip.map(|a| a.ip()), Some(OTHER_IP));
        assert_eq!(reused, Some(freed));
    }
}
