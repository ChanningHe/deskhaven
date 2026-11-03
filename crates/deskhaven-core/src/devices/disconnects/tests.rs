use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::clock::ManualClock;

const T0_MS: i64 = 1_700_000_000_000;
const ID: &str = "123456789";

fn queue() -> (DisconnectQueue, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::at(T0_MS));
    (DisconnectQueue::new(clock.clone()), clock)
}

#[test]
fn only_requested_ids_the_device_still_reports_are_handed_out_once() {
    // Arrange
    let (q, _) = queue();
    q.disconnect(ID, &[12, 13]).unwrap();

    // Act
    let first = q.take(ID, &[11, 12, 14]);
    let second = q.take(ID, &[11, 12, 14]);

    // Assert
    assert_eq!(first, vec![12], "13 is gone, 11 and 14 were not asked for");
    assert!(second.is_empty(), "handed out once");
    assert!(!q.is_pending(ID));
}

#[test]
fn a_heartbeat_without_live_connections_drops_the_request() {
    // Arrange
    let (q, _) = queue();
    q.disconnect(ID, &[12]).unwrap();

    // Act
    let none = q.take(ID, &[]);
    let later = q.take(ID, &[12]);

    // Assert
    assert!(none.is_empty() && later.is_empty());
}

#[test]
fn disconnect_all_covers_every_reported_connection_once_each() {
    // Arrange
    let (q, _) = queue();
    q.disconnect(ID, &[12]).unwrap();
    q.disconnect_all(ID).unwrap();
    q.disconnect("987654321", &[1]).unwrap();

    // Act
    let all = q.take(ID, &[5, 7, 5]);

    // Assert
    assert_eq!(all, vec![5, 7]);
    assert!(q.is_pending("987654321"), "per device");
}

#[test]
fn requests_merge_and_all_absorbs_specific_ids() {
    // Arrange
    let (q, _) = queue();
    q.disconnect(ID, &[1]).unwrap();
    q.disconnect(ID, &[2]).unwrap();
    q.disconnect("987654321", &[1]).unwrap();
    q.disconnect_all("987654321").unwrap();
    q.disconnect("987654321", &[1]).unwrap();

    // Act
    let merged = q.take(ID, &[1, 2, 3]);
    let all = q.take("987654321", &[1, 2, 3]);

    // Assert
    assert_eq!(merged, vec![1, 2]);
    assert_eq!(all, vec![1, 2, 3]);
}

#[test]
fn a_request_expires_when_not_picked_up_in_time() {
    // Arrange
    let (q, clock) = queue();
    q.disconnect(ID, &[12]).unwrap();
    q.disconnect("987654321", &[12]).unwrap();

    // Act
    clock.advance(DISCONNECT_TTL - Duration::from_millis(1));
    let in_time = q.take("987654321", &[12]);
    clock.advance(Duration::from_millis(1));
    let late = q.take(ID, &[12]);

    // Assert
    assert_eq!(in_time, vec![12]);
    assert!(late.is_empty(), "a restarted client may reuse the id");
}

#[test]
fn ids_per_device_are_capped() {
    // Arrange
    let (q, _) = queue();
    let many: Vec<i32> = (0..200).collect();

    // Act
    q.disconnect(ID, &many).unwrap();
    let taken = q.take(ID, &many);

    // Assert
    assert_eq!(taken.len(), MAX_CONNS);
}

#[test]
fn a_full_queue_refuses_new_devices_until_requests_expire() {
    // Arrange
    let (q, clock) = queue();
    for i in 0..MAX_QUEUED_DEVICES {
        q.disconnect(&i.to_string(), &[1]).unwrap();
    }

    // Act
    let refused = q.disconnect_all(ID);
    let merged = q.disconnect("0", &[2]);
    clock.advance(DISCONNECT_TTL);
    let after_expiry = q.disconnect_all(ID);

    // Assert
    assert_eq!(refused, Err(QueueFull::Devices));
    assert_eq!(merged, Ok(()), "a device already queued can still add ids");
    assert_eq!(after_expiry, Ok(()));
}
