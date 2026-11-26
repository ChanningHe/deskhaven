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
fn only_live_requested_ids_are_handed_out_once_and_the_rest_wait() {
    let (q, _) = queue();
    q.disconnect(ID, &[12, 13]).unwrap();

    let first = q.take(ID, &[11, 12, 14]);
    let second = q.take(ID, &[11, 12, 14]);
    let pending = q.is_pending(ID);
    let later = q.take(ID, &[13]);

    assert_eq!(first, vec![12], "11 and 14 were not asked for");
    assert!(second.is_empty(), "12 handed out once");
    assert!(pending, "13 was not reported, so it waits");
    assert_eq!(later, vec![13]);
    assert!(!q.is_pending(ID));
}

#[test]
fn a_heartbeat_without_live_connections_keeps_the_request() {
    let (q, _) = queue();
    q.disconnect(ID, &[12]).unwrap();
    q.disconnect_all("987654321").unwrap();

    let none = q.take(ID, &[]);
    let later = q.take(ID, &[12]);
    let none_all = q.take("987654321", &[]);
    let later_all = q.take("987654321", &[3]);

    assert!(none.is_empty() && none_all.is_empty());
    assert_eq!((later, later_all), (vec![12], vec![3]));
}

#[test]
fn merging_does_not_extend_the_deadline_of_older_ids() {
    let (q, clock) = queue();
    q.disconnect(ID, &[1]).unwrap();
    clock.advance(DISCONNECT_TTL / 2);
    q.disconnect(ID, &[1, 2]).unwrap();

    clock.advance(DISCONNECT_TTL / 2);
    let taken = q.take(ID, &[1, 2]);

    assert_eq!(taken, vec![2], "1 expired on its own deadline");
}

#[test]
fn disconnect_all_covers_every_reported_connection_once_each() {
    let (q, _) = queue();
    q.disconnect(ID, &[12]).unwrap();
    q.disconnect_all(ID).unwrap();
    q.disconnect("987654321", &[1]).unwrap();

    let all = q.take(ID, &[5, 7, 5]);

    assert_eq!(all, vec![5, 7]);
    assert_eq!(q.take(ID, &[5, 7]), Vec::<i32>::new(), "all used up");
    assert_eq!(
        q.take(ID, &[12]),
        vec![12],
        "12 was never reported: it waited"
    );
    assert!(q.is_pending("987654321"), "per device");
}

#[test]
fn requests_merge_and_all_absorbs_specific_ids() {
    let (q, _) = queue();
    q.disconnect(ID, &[1]).unwrap();
    q.disconnect(ID, &[2]).unwrap();
    q.disconnect("987654321", &[1]).unwrap();
    q.disconnect_all("987654321").unwrap();
    q.disconnect("987654321", &[1]).unwrap();

    let merged = q.take(ID, &[1, 2, 3]);
    let all = q.take("987654321", &[1, 2, 3]);

    assert_eq!(merged, vec![1, 2]);
    assert_eq!(all, vec![1, 2, 3]);
}

#[test]
fn a_request_expires_when_not_picked_up_in_time() {
    let (q, clock) = queue();
    q.disconnect(ID, &[12]).unwrap();
    q.disconnect("987654321", &[12]).unwrap();

    clock.advance(DISCONNECT_TTL - Duration::from_millis(1));
    let in_time = q.take("987654321", &[12]);
    clock.advance(Duration::from_millis(1));
    let late = q.take(ID, &[12]);

    assert_eq!(in_time, vec![12]);
    assert!(late.is_empty(), "a restarted client may reuse the id");
}

#[test]
fn ids_per_device_are_capped() {
    let (q, _) = queue();
    let many: Vec<i32> = (0..200).collect();

    q.disconnect(ID, &many).unwrap();
    let taken = q.take(ID, &many);

    assert_eq!(taken.len(), MAX_CONNS);
}

#[test]
fn a_full_queue_refuses_new_devices_until_requests_expire() {
    let (q, clock) = queue();
    for i in 0..MAX_QUEUED_DEVICES {
        q.disconnect(&i.to_string(), &[1]).unwrap();
    }

    let refused = q.disconnect_all(ID);
    let merged = q.disconnect("0", &[2]);
    clock.advance(DISCONNECT_TTL);
    let after_expiry = q.disconnect_all(ID);

    assert_eq!(refused, Err(QueueFull::Devices));
    assert_eq!(merged, Ok(()), "a device already queued can still add ids");
    assert_eq!(after_expiry, Ok(()));
}

#[test]
fn a_wall_clock_step_back_does_not_extend_a_request() {
    let (q, clock) = queue();
    q.disconnect(ID, &[12]).unwrap();

    clock.step_wall(-3_600_000);
    clock.advance(DISCONNECT_TTL);
    let late = q.take(ID, &[12]);

    assert!(late.is_empty());
}
