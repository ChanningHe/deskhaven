//! Millisecond timestamp convention.

use deskhaven_db::time;

#[test]
fn now_round_trips_through_millis() {
    let now = time::now();

    let back = time::from_millis(now.timestamp_millis()).unwrap();

    assert_eq!(back, now);
}
