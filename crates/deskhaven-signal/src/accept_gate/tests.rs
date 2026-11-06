#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::Ipv4Addr;

use super::*;

const LIMITS: AcceptLimits = AcceptLimits {
    per_ip_burst: 4,
    per_ip_per_second: 2,
    per_ip_pending: 3,
    per_net_pending: 100,
    per_ip_connections: 100,
    per_net_connections: 100,
    pending: 5,
    ws_pending: 5,
    connections: 100,
};

fn v4(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

fn nth(n: u32) -> IpAddr {
    IpAddr::V4(Ipv4Addr::from(0x0A00_0000 + n))
}

fn v6(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn evicted(slot: &Slot) -> bool {
    slot.evicted().is_cancelled()
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
fn a_source_has_at_most_per_ip_pending_slots_until_one_is_established() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let held: Vec<_> = (0..3).map(|_| gate.admit(v4(1), t0).unwrap()).collect();

    assert_eq!(gate.admit(v4(1), t0).err(), Some(Refused::SourcePending));
    held[0].established();
    assert!(gate.admit(v4(1), t0).is_ok());
}

#[test]
fn a_source_has_at_most_per_ip_connections_established_or_not() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_connections: 2,
        ..LIMITS
    });
    let t0 = Instant::now();
    let held: Vec<_> = (0..2).map(|_| gate.admit(v4(1), t0).unwrap()).collect();
    held.iter().for_each(Slot::established);

    assert_eq!(
        gate.admit(v4(1), t0).err(),
        Some(Refused::SourceConnections)
    );
    drop(held);
    assert!(gate.admit(v4(1), t0).is_ok());
}

#[test]
fn at_the_pending_cap_the_heaviest_sources_oldest_pending_connection_is_evicted() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let light = gate.admit(v4(1), t0).unwrap();
    let heavy: Vec<_> = (0..3).map(|_| gate.admit(v4(2), t0).unwrap()).collect();
    let other = gate.admit(v4(3), t0).unwrap();

    let fresh = gate.admit(v4(9), t0).unwrap();

    assert!(evicted(&heavy[0]), "the oldest of the heaviest source");
    assert!(!evicted(&light) && !evicted(&other) && !evicted(&fresh));
    assert!(heavy[1..].iter().all(|s| !evicted(s)));
    drop(heavy);
    assert_eq!(
        gate.lock().pending_len(Port::Main),
        3,
        "an evicted slot is released once"
    );
}

#[test]
fn at_the_connection_cap_pending_then_idle_then_peers_go_never_the_new_one() {
    let gate = AcceptGate::new(AcceptLimits {
        connections: 3,
        ..LIMITS
    });
    let t0 = Instant::now();
    let peer = gate.admit(nth(1), t0).unwrap();
    peer.active(true);
    let idle = gate.admit(nth(2), t0).unwrap();
    idle.established();
    let pending = gate.admit(nth(3), t0).unwrap();

    let first = gate.admit(nth(4), t0).unwrap();
    assert!(evicted(&pending), "a pending handshake first");
    first.active(false);
    let second = gate.admit(nth(5), t0).unwrap();
    assert!(evicted(&idle), "then the idle connection idle the longest");
    second.active(false);
    first.active(false);
    let third = gate.admit(nth(6), t0).unwrap();

    assert!(evicted(&second), "the longest idle of the newer ones");
    assert!(!evicted(&peer) && !evicted(&first) && !evicted(&third));
}

#[test]
fn a_websocket_connection_has_its_own_pending_budget() {
    let gate = AcceptGate::new(AcceptLimits {
        ws_pending: 2,
        ..LIMITS
    });
    let t0 = Instant::now();
    let ws: Vec<_> = (1..=3)
        .map(|n| gate.admit_ws(nth(n), false, t0).unwrap())
        .collect();
    let main: Vec<_> = (10..15).map(|n| gate.admit(nth(n), t0).unwrap()).collect();

    assert!(evicted(&ws[0]) && !evicted(&ws[2]));
    assert!(
        main.iter().all(|s| !evicted(s)),
        "the main port's budget is its own"
    );
}

#[test]
fn a_trusted_proxy_skips_the_per_source_pending_limits_only() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_pending: 1,
        ws_pending: 3,
        ..LIMITS
    });
    let t0 = Instant::now();
    let proxy = v4(7);
    let held: Vec<_> = (0..4)
        .map(|_| gate.admit_ws(proxy, true, t0).unwrap())
        .collect();

    assert!(evicted(&held[0]), "still under the websocket budget");
    let _client = gate.admit_ws(v4(8), false, t0).unwrap();
    assert_eq!(
        gate.admit_ws(v4(8), false, t0).err(),
        Some(Refused::SourcePending)
    );
}

#[test]
fn a_websocket_connection_counts_per_source_once_bound() {
    let gate = AcceptGate::new(AcceptLimits {
        per_ip_connections: 1,
        ..LIMITS
    });
    let t0 = Instant::now();
    let first = gate.admit_ws(v4(7), true, t0).unwrap();
    first.established();
    assert!(first.bind_source(v4(1)));

    let second = gate.admit_ws(v4(7), true, t0).unwrap();
    assert!(!second.bind_source(v4(1)));
    assert!(second.bind_source(v4(2)));
    assert_eq!(
        gate.admit(v4(1), t0).err(),
        Some(Refused::SourceConnections)
    );
    drop(first);
    assert!(gate.admit(v4(1), t0).is_ok());
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
fn an_ipv6_slash_48_has_its_own_pending_and_connection_caps() {
    let gate = AcceptGate::new(AcceptLimits {
        per_net_pending: 3,
        per_net_connections: 4,
        ..LIMITS
    });
    let t0 = Instant::now();
    let in_net = |n: u32| v6(&format!("2001:db8:0:{n}::1"));
    let held: Vec<_> = (1..=3)
        .map(|n| gate.admit(in_net(n), t0).unwrap())
        .collect();

    assert_eq!(gate.admit(in_net(9), t0).err(), Some(Refused::NetPending));
    assert!(gate.admit(v6("2001:db8:1:9::1"), t0).is_ok(), "another /48");
    held.iter().for_each(Slot::established);
    let fourth = gate.admit(in_net(9), t0).unwrap();
    assert_eq!(
        gate.admit(in_net(10), t0).err(),
        Some(Refused::NetConnections)
    );
    drop((held, fourth));
    assert_eq!(gate.lock().nets(), 0);
}

#[test]
fn prune_forgets_only_idle_sources_with_a_full_bucket() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    let held = gate.admit(v4(1), t0).unwrap();
    drop(gate.admit(v4(2), t0).unwrap());
    let established = gate.admit(v4(3), t0).unwrap();
    established.established();

    gate.prune(t0);
    assert_eq!(gate.lock().tracked(), 3);
    gate.prune(t0 + Duration::from_secs(1));
    assert_eq!(
        gate.lock().tracked(),
        2,
        "v4(2) is idle; v4(3) still has a connection"
    );
    drop((held, established));
    gate.prune(t0 + Duration::from_secs(1));
    assert_eq!(gate.lock().tracked(), 0);
}

#[test]
fn the_global_caps_scale_with_the_open_file_limit_within_bounds() {
    let default = AcceptLimits::default();
    assert_eq!(AcceptLimits::for_open_files(None), default);
    assert_eq!(AcceptLimits::for_open_files(Some(65_536)), default);

    let big = AcceptLimits::for_open_files(Some(1 << 20));
    assert_eq!(
        (big.connections, big.pending, big.ws_pending),
        (32_768, 8192, 4096)
    );
    assert_eq!(big.per_ip_connections, default.per_ip_connections);

    let tiny = AcceptLimits::for_open_files(Some(64));
    assert_eq!(
        (tiny.connections, tiny.pending, tiny.ws_pending),
        (64, 16, 8)
    );
}

/// Re-review MEDIUM-2: at a low open-file limit the scaled caps must not let the connection
/// cap refuse before pending eviction runs.
#[test]
fn a_low_open_file_limit_cannot_reopen_the_pending_lockout() {
    for nofile in [1024, 4096] {
        let limits = AcceptLimits::for_open_files(Some(nofile));
        assert!(
            limits.pending <= limits.connections / 2,
            "{nofile}: {limits:?}"
        );
        let gate = AcceptGate::new(limits);
        let t0 = Instant::now();
        let held: Vec<_> = (0..limits.connections)
            .map(|n| gate.admit(nth(n), t0).unwrap())
            .collect();

        let fresh = gate.admit(v4(1), t0);

        assert!(fresh.is_ok(), "{nofile}: {:?}", fresh.err());
        drop(held);
    }
}

/// Re-review LOW: a full source table forgets idle sources instead of skipping the checks.
#[test]
fn a_full_source_table_still_applies_the_per_source_limits() {
    let gate = AcceptGate::new(LIMITS);
    let t0 = Instant::now();
    for n in 0..100_000 {
        assert!(gate.lock().track(nth(n), &LIMITS, t0));
    }
    let later = t0 + Duration::from_secs(1);
    let held: Vec<_> = (0..3).map(|_| gate.admit(v4(1), later).unwrap()).collect();

    assert_eq!(gate.admit(v4(1), later).err(), Some(Refused::SourcePending));
    assert!(gate.lock().tracked() <= 100_000);
    drop(held);
}
