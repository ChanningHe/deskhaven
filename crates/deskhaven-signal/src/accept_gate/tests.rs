#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::Ipv4Addr;

use super::*;

const LIMITS: AcceptLimits = AcceptLimits {
    per_ip_burst: 4,
    per_ip_per_second: 2,
    per_ip_pending: 3,
    pending: 5,
    per_net_pending: 100,
    per_ip_connections: 100,
    connections: 100,
};

fn v4(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

fn v6(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn tracked(gate: &AcceptGate) -> usize {
    gate.lock().sources.len()
}

#[test]
fn a_source_gets_its_burst_then_the_refill_rate() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    for _ in 0..4 {
        drop(gate.admit(v4(1), t0).unwrap());
    }

    assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::Rate));
    assert!(gate.admit(v4(2), t0).is_ok());
    let later = t0 + Duration::from_millis(500);
    drop(gate.admit(v4(1), later).unwrap());
    assert_eq!(gate.admit(v4(1), later).err(), Some(Refused::Rate));
}

#[test]
fn a_source_has_at_most_per_ip_pending_slots_until_one_is_released() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let held: Vec<_> = (0..3).map(|_| gate.admit(v4(1), t0).unwrap()).collect();

    assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::SourcePending));
    drop(held);
    assert!(gate.admit(v4(1), t0).is_ok());
}

#[test]
fn a_first_frame_releases_the_pending_slot_but_keeps_the_connection() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_connections: 2,
        ..LIMITS
    });
    let t0 = Instant::now();
    let held: Vec<_> = (0..2)
        .map(|_| gate.admit(v4(1), t0).unwrap().connection)
        .collect();

    assert_eq!(
        gate.admit(v4(1), t0).err(),
        Some(Refused::SourceConnections)
    );
    drop(held);
    assert!(gate.admit(v4(1), t0).is_ok());
}

#[test]
fn at_the_global_pending_cap_the_oldest_pending_connection_is_evicted() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let held: Vec<_> = (1..=5).map(|n| gate.admit(v4(n), t0).unwrap()).collect();

    let fresh = gate.admit(v4(9), t0).unwrap();

    assert!(held[0].pending.evicted().is_cancelled());
    assert!(
        held[1..]
            .iter()
            .all(|a| !a.pending.evicted().is_cancelled())
    );
    assert!(!fresh.pending.evicted().is_cancelled());
    assert_eq!(gate.lock().pending.len(), 5);
    drop(held);
    assert_eq!(
        gate.lock().pending.len(),
        1,
        "an evicted slot is not released twice"
    );
}

#[test]
fn the_global_connection_cap_refuses_on_both_ports() {
    let gate = AcceptGate::new(AcceptLimits {
        connections: 2,
        ..LIMITS
    });
    let t0 = Instant::now();
    let main = gate.admit(v4(1), t0).unwrap().connection;
    let ws = gate.open().unwrap();

    assert_eq!(gate.admit(v4(2), t0).err(), Some(Refused::Connections));
    assert_eq!(gate.open().err(), Some(Refused::Connections));
    drop((main, ws));
    assert!(gate.open().is_ok());
}

#[test]
fn a_websocket_connection_counts_per_source_once_bound() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_connections: 1,
        ..LIMITS
    });
    let mut first = gate.open().unwrap();
    assert!(first.bind_source(&gate, v4(1)));

    let mut second = gate.open().unwrap();
    assert!(!second.bind_source(&gate, v4(1)));
    assert!(second.bind_source(&gate, v4(2)));
    assert_eq!(
        gate.admit(v4(1), Instant::now()).err(),
        Some(Refused::SourceConnections)
    );
    drop(first);
    assert!(gate.admit(v4(1), Instant::now()).is_ok());
}

#[test]
fn a_refusal_takes_neither_a_token_nor_a_slot() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_pending: 1,
        ..LIMITS
    });
    let t0 = Instant::now();
    let held = gate.admit(v4(1), t0).unwrap();
    for _ in 0..10 {
        assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::SourcePending));
    }
    drop(held);

    let left = (0..10).filter(|_| gate.admit(v4(1), t0).is_ok()).count();

    assert_eq!(left, 3);
}

#[test]
fn ipv6_sources_share_their_slash_64_and_mapped_ipv4_is_ipv4() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let _held: Vec<_> = ["2001:db8::1", "2001:db8::2", "2001:db8::ffff:1"]
        .into_iter()
        .map(|ip| gate.admit(v6(ip), t0).unwrap())
        .collect();

    assert_eq!(
        gate.admit(v6("2001:db8:0:0:1::1"), t0).err(),
        Some(Refused::SourcePending)
    );
    assert!(gate.admit(v6("2001:db8:0:1::1"), t0).is_ok());
    assert_eq!(source_key(v6("::ffff:192.0.2.1")), v4(1));
}

#[test]
fn an_ipv6_slash_48_has_at_most_per_net_pending_across_its_slash_64s() {
    let gate = AcceptGate::new(AcceptLimits {
        per_net_pending: 3,
        ..LIMITS
    });
    let t0 = Instant::now();
    let held: Vec<_> = (1..=3)
        .map(|n| gate.admit(v6(&format!("2001:db8:0:{n}::1")), t0).unwrap())
        .collect();

    assert_eq!(
        gate.admit(v6("2001:db8:0:9::1"), t0).err(),
        Some(Refused::NetPending)
    );
    assert!(gate.admit(v6("2001:db8:1:9::1"), t0).is_ok(), "another /48");
    drop(held);
    assert!(gate.lock().nets.len() <= 1);
    assert!(gate.admit(v6("2001:db8:0:9::1"), t0).is_ok());
}

#[test]
fn prune_forgets_only_idle_sources_with_a_full_bucket() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let held = gate.admit(v4(1), t0).unwrap();
    drop(gate.admit(v4(2), t0).unwrap());
    let established = gate.admit(v4(3), t0).unwrap().connection;

    gate.prune(t0);
    assert_eq!(tracked(&gate), 3);
    gate.prune(t0 + Duration::from_secs(1));
    assert_eq!(
        tracked(&gate),
        2,
        "v4(2) is idle; v4(3) still has a connection"
    );
    drop((held, established));
    gate.prune(t0 + Duration::from_secs(1));
    assert_eq!(tracked(&gate), 0);
}

#[test]
fn the_global_caps_scale_with_the_open_file_limit() {
    let default = AcceptLimits::default();
    assert_eq!(AcceptLimits::for_open_files(None), default);

    let big = AcceptLimits::for_open_files(Some(1 << 20));
    assert_eq!((big.connections, big.pending), (1 << 18, 1 << 16));
    assert_eq!(big.per_ip_connections, default.per_ip_connections);

    let small = AcceptLimits::for_open_files(Some(1024));
    assert_eq!((small.connections, small.pending), (1024, 2048));
}
