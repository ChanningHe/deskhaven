//! Millisecond timestamp convention.

use deskhaven_db::time;

#[test]
fn now_has_millisecond_precision() {
    let now = time::now();

    assert_eq!(now.timestamp_subsec_nanos() % 1_000_000, 0);
}

#[test]
fn now_round_trips_through_millis() {
    let now = time::now();

    let back = time::from_millis(now.timestamp_millis()).unwrap();

    assert_eq!(back, now);
}

#[test]
fn from_millis_rejects_out_of_range() {
    assert!(time::from_millis(i64::MAX).is_none());
}

#[test]
fn from_millis_is_utc_epoch_based() {
    let ts = time::from_millis(0).unwrap();

    assert_eq!(ts.to_rfc3339(), "1970-01-01T00:00:00+00:00");
}
