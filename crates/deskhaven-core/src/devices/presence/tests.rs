use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::clock::ManualClock;

const T0_MS: i64 = 1_700_000_000_000;
const ID: &str = "123456789";

fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

fn presence(capacity: usize) -> (Presence, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::at(T0_MS));
    (Presence::with_capacity(clock.clone(), capacity), clock)
}

fn just_under(ttl: Duration) -> Duration {
    ttl - Duration::from_millis(1)
}

#[test]
fn a_signal_registration_is_online_for_the_reg_timeout() {
    // Arrange
    let (p, clock) = presence(10);

    // Act
    p.signal_online(ID, ip(1));
    clock.advance(just_under(SIGNAL_TTL));
    let before = p.get(ID).unwrap();
    clock.advance(Duration::from_millis(1));
    let after = p.get(ID).unwrap();

    // Assert
    assert!(before.online);
    assert!(!after.online, "REG_TIMEOUT elapsed");
    assert_eq!(after.last_seen_ms, Some(T0_MS));
    assert_eq!(after.ip, Some(ip(1)));
}

#[test]
fn a_heartbeat_is_online_for_three_heartbeat_periods_and_keeps_its_conns() {
    // Arrange
    let (p, clock) = presence(10);

    // Act
    p.heartbeat(ID, ip(2), &[12, 13]);
    clock.advance(just_under(HEARTBEAT_TTL));
    let before = p.get(ID).unwrap();
    clock.advance(Duration::from_millis(1));

    // Assert
    assert!(before.online);
    assert_eq!(before.conns, [12, 13]);
    assert!(!p.is_online(ID));
}

#[test]
fn either_source_keeps_a_device_online_and_the_latest_contact_wins() {
    // Arrange
    let (p, clock) = presence(10);
    p.heartbeat(ID, ip(2), &[]);

    // Act
    clock.advance(Duration::from_secs(40));
    p.signal_online(ID, ip(3));
    clock.advance(Duration::from_secs(10));
    let status = p.get(ID).unwrap();

    // Assert
    assert!(status.online, "heartbeat expired, signal still fresh");
    assert_eq!(status.last_seen_ms, Some(T0_MS + 40_000));
    assert_eq!(status.ip, Some(ip(3)));
}

#[test]
fn a_closed_signal_channel_ends_signal_liveness_but_not_a_heartbeat() {
    // Arrange
    let (p, _) = presence(10);
    p.signal_online(ID, ip(1));
    p.signal_online("other", ip(4));
    p.heartbeat("other", ip(4), &[]);

    // Act
    p.signal_offline(ID);
    p.signal_offline("other");
    p.signal_offline("never-seen");

    // Assert
    assert!(!p.is_online(ID));
    assert!(p.is_online("other"));
    assert_eq!(p.get("never-seen"), None, "offline creates nothing");
    assert_eq!(p.online_count(), 1);
}

#[test]
fn conns_are_capped_and_replaced_by_each_heartbeat() {
    // Arrange
    let (p, _) = presence(10);
    let many: Vec<i32> = (0..200).collect();

    // Act
    p.heartbeat(ID, ip(1), &many);
    let capped = p.get(ID).unwrap().conns.len();
    p.heartbeat(ID, ip(1), &[7]);

    // Assert
    assert_eq!(capped, MAX_CONNS);
    assert_eq!(p.get(ID).unwrap().conns, [7]);
}

#[test]
fn take_unflushed_reports_each_seen_device_once_then_evicts_offline_ones() {
    // Arrange
    let (p, clock) = presence(10);
    p.signal_online("a", ip(1));
    p.heartbeat("b", ip(2), &[]);
    clock.advance(Duration::from_secs(5));
    p.signal_online("a", ip(3));

    // Act
    let mut first = p.take_unflushed();
    first.sort_by(|x, y| x.rustdesk_id.cmp(&y.rustdesk_id));
    let second = p.take_unflushed();
    clock.advance(HEARTBEAT_TTL);
    let third = p.take_unflushed();

    // Assert
    let seen = |id: &str, ms: i64, last: u8| Seen {
        rustdesk_id: id.to_owned(),
        last_seen_ms: ms,
        ip: Some(ip(last)),
    };
    assert_eq!(first, [seen("a", T0_MS + 5_000, 3), seen("b", T0_MS, 2)]);
    assert!(second.is_empty(), "nothing new");
    assert!(third.is_empty());
    assert!(p.is_empty(), "offline and flushed entries are evicted");
}

#[test]
fn a_full_table_evicts_offline_flushed_entries_to_admit_a_new_device() {
    // Arrange
    let (p, clock) = presence(2);
    p.signal_online("a", ip(1));
    p.signal_online("b", ip(2));
    p.take_unflushed();
    clock.advance(SIGNAL_TTL);

    // Act
    p.signal_online("c", ip(3));

    // Assert
    assert!(p.is_online("c"));
    assert_eq!(p.len(), 1);
}

#[test]
fn a_full_table_of_online_devices_refuses_new_ids_but_keeps_updating_known_ones() {
    // Arrange
    let (p, clock) = presence(2);
    p.signal_online("a", ip(1));
    p.signal_online("b", ip(2));

    // Act
    p.signal_online("c", ip(3));
    clock.advance(Duration::from_secs(20));
    p.heartbeat("a", ip(1), &[1]);

    // Assert
    assert_eq!(p.get("c"), None);
    assert_eq!(p.len(), 2);
    assert_eq!(p.get("a").unwrap().last_seen_ms, Some(T0_MS + 20_000));
}

/// Without a flush (M2-14 not wired yet), a full table still admits new devices: offline
/// entries go oldest contact first, unflushed or not; online ones stay.
#[test]
fn a_full_table_evicts_offline_entries_oldest_first_even_unflushed() {
    // Arrange
    let (p, clock) = presence(3);
    p.signal_online("oldest", ip(1));
    clock.advance(Duration::from_secs(1));
    p.signal_online("older", ip(2));
    clock.advance(SIGNAL_TTL);
    p.signal_online("online", ip(3));

    // Act
    p.signal_online("new", ip(4));

    // Assert
    assert_eq!(p.get("oldest"), None, "oldest offline entry evicted");
    assert!(p.get("older").is_some(), "only as many as needed");
    assert!(p.is_online("online") && p.is_online("new"));
    assert_eq!(p.len(), 3);
}

/// Online entries are never evicted: a table full of them refuses new ids.
#[test]
fn a_full_table_keeps_online_entries_over_offline_unflushed_ones() {
    // Arrange
    let (p, clock) = presence(2);
    p.signal_online("stale", ip(1));
    clock.advance(SIGNAL_TTL);
    p.signal_online("live", ip(2));

    // Act
    p.signal_online("new", ip(3));
    p.signal_online("newer", ip(4));

    // Assert
    assert_eq!(p.get("stale"), None);
    assert!(p.is_online("live") && p.is_online("new"));
    assert_eq!(
        p.get("newer"),
        None,
        "only online entries left, sweep rate-limited"
    );
}

fn identity() -> Identity {
    Identity {
        device_id: 7,
        uuid: b"uuid".to_vec(),
        has_sysinfo: false,
    }
}

#[test]
fn identity_is_cached_until_forgotten() {
    // Arrange
    let (p, _) = presence(10);

    // Act
    p.remember(ID, identity(), p.identity_epoch());
    let cached = p.identity(ID);
    p.forget(ID);

    // Assert
    assert_eq!(cached, Some(identity()));
    assert_eq!(p.identity(ID), None);
    assert_eq!(p.get(ID), None);
}

/// A lookup that read the row before the device was deleted cannot cache it afterwards.
#[test]
fn an_identity_read_before_a_forget_is_not_cached() {
    // Arrange
    let (p, _) = presence(10);
    let epoch = p.identity_epoch();

    // Act: the device is deleted while the lookup is in flight.
    p.forget(ID);
    p.remember(ID, identity(), epoch);

    // Assert
    assert_eq!(p.identity(ID), None);
    assert_eq!(p.get(ID), None);
}

#[test]
fn a_cached_identity_alone_is_not_online_and_is_evictable() {
    // Arrange
    let (p, _) = presence(1);
    let identity = Identity {
        device_id: 7,
        uuid: b"uuid".to_vec(),
        has_sysinfo: false,
    };
    p.remember(ID, identity, p.identity_epoch());

    // Act
    let status = p.get(ID).unwrap();
    p.signal_online("other", ip(1));

    // Assert
    assert!(!status.online);
    assert_eq!(status.last_seen_ms, None);
    assert!(p.is_online("other"), "the identity-only entry made room");
}

#[test]
fn a_stored_sysinfo_is_marked_on_the_cached_identity_only() {
    // Arrange
    let (p, _) = presence(10);
    p.remember(ID, identity(), p.identity_epoch());

    // Act
    p.mark_sysinfo(ID);
    p.mark_sysinfo("unknown");

    // Assert
    assert!(p.identity(ID).unwrap().has_sysinfo);
    assert_eq!(p.get("unknown"), None, "marking never creates an entry");
}

#[test]
fn heartbeats_closer_than_the_minimum_interval_are_refused_without_effect() {
    // Arrange
    let (p, clock) = presence(10);
    let min = Duration::from_secs(2);

    // Act
    let first = p.try_heartbeat(ID, ip(1), &[1], min);
    clock.advance(just_under(min));
    let early = p.try_heartbeat(ID, ip(2), &[2], min);
    let early_state = p.get(ID).unwrap();
    clock.advance(Duration::from_millis(1));
    let on_time = p.try_heartbeat(ID, ip(3), &[3], min);
    let other = p.try_heartbeat("other", ip(4), &[], min);

    // Assert
    assert!(first && on_time && other);
    assert!(!early);
    assert_eq!(
        early_state.conns,
        vec![1],
        "a refused heartbeat changes nothing"
    );
    assert_eq!(early_state.ip, Some(ip(1)));
    let state = p.get(ID).unwrap();
    assert_eq!((state.conns, state.ip), (vec![3], Some(ip(3))));
    assert_eq!(state.last_seen_ms, Some(T0_MS + 2_000));
}

#[test]
fn a_signal_registration_does_not_count_against_the_heartbeat_interval() {
    // Arrange
    let (p, _) = presence(10);
    p.signal_online(ID, ip(1));

    // Act
    let accepted = p.try_heartbeat(ID, ip(1), &[], Duration::from_secs(2));

    // Assert
    assert!(accepted);
}

/// Liveness and the heartbeat interval run on the monotonic clock: a wall clock stepped back
/// an hour neither refuses every heartbeat nor keeps silent devices online.
#[test]
fn a_wall_clock_step_back_neither_refuses_heartbeats_nor_keeps_devices_online() {
    // Arrange
    let (p, clock) = presence(10);
    let min = Duration::from_secs(2);
    p.try_heartbeat(ID, ip(1), &[], min);
    p.signal_online("signal", ip(2));

    // Act
    clock.step_wall(-3_600_000);
    clock.advance(min);
    let accepted = p.try_heartbeat(ID, ip(1), &[], min);
    let seen = p.get(ID).unwrap().last_seen_ms;
    clock.advance(HEARTBEAT_TTL);
    let still_online = p.is_online(ID) || p.is_online("signal");

    // Assert
    assert!(accepted);
    assert_eq!(
        seen,
        Some(T0_MS - 3_600_000 + 2_000),
        "stored times stay wall times"
    );
    assert!(!still_online);
}

#[test]
fn live_counts_the_online_devices_and_their_connections_only() {
    // Arrange
    let (p, clock) = presence(10);
    p.heartbeat("a", ip(1), &[1, 2, 3]);
    p.signal_online("b", ip(2));
    clock.advance(just_under(SIGNAL_TTL));
    p.heartbeat("c", ip(3), &[9]);

    // Act
    let all = p.live();
    clock.advance(Duration::from_secs(1));
    // "b" has no heartbeat and its registration expired; "a" and "c" still beat.
    let later = p.live();
    clock.advance(HEARTBEAT_TTL);
    let none = p.live();

    // Assert
    assert_eq!(
        all,
        LiveCounts {
            devices: 3,
            connections: 4
        }
    );
    assert_eq!(
        later,
        LiveCounts {
            devices: 2,
            connections: 4
        }
    );
    assert_eq!(none, LiveCounts::default());
}

#[test]
fn a_stale_heartbeats_conns_stop_counting_while_the_signal_keeps_the_device_online() {
    // Arrange
    let (p, clock) = presence(10);
    p.heartbeat(ID, ip(1), &[4, 5]);
    p.signal_online(ID, ip(1));
    clock.advance(just_under(HEARTBEAT_TTL));
    p.signal_online(ID, ip(1));
    let fresh = (p.get(ID).unwrap().conns, p.live());

    // Act
    clock.advance(Duration::from_millis(1));
    let stale = (p.get(ID).unwrap(), p.live());

    // Assert
    assert_eq!(
        fresh,
        (
            vec![4, 5],
            LiveCounts {
                devices: 1,
                connections: 2
            }
        )
    );
    assert!(stale.0.online, "the signal registration is still fresh");
    assert!(stale.0.conns.is_empty(), "the last heartbeat expired");
    assert_eq!(
        stale.1,
        LiveCounts {
            devices: 1,
            connections: 0
        }
    );
}
