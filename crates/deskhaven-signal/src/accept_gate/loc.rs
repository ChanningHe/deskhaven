//! Source keys of the accept gate (M4-05): IPv4 addresses and IPv6 /64s, IPv6 /48s.

use std::net::{IpAddr, Ipv6Addr};

/// Where a connection is counted: its source and, for IPv6, its /48.
#[derive(Clone, Copy)]
pub(super) struct Loc {
    pub(super) key: IpAddr,
    pub(super) net: Option<IpAddr>,
}

impl Loc {
    pub(super) fn of(ip: IpAddr) -> Self {
        Self {
            key: source_key(ip),
            net: net_key(ip),
        }
    }
}

/// The address itself for IPv4 (IPv4-mapped IPv6 included), its /64 for IPv6.
pub(crate) fn source_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => prefix(v6, 64),
        v4 => v4,
    }
}

/// The /48 of an IPv6 address; `None` for IPv4.
fn net_key(ip: IpAddr) -> Option<IpAddr> {
    match ip.to_canonical() {
        IpAddr::V6(v6) => Some(prefix(v6, 48)),
        IpAddr::V4(_) => None,
    }
}

fn prefix(ip: Ipv6Addr, len: u32) -> IpAddr {
    let mask = u128::MAX.checked_shl(128 - len).unwrap_or(0);
    IpAddr::V6(Ipv6Addr::from(u128::from(ip) & mask))
}
