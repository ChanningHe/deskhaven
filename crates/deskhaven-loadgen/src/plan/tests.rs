use super::*;

const QUIET: Rates = Rates {
    connections_per_hour: 0.0,
    sysinfo_per_hour: 0.0,
};
/// A connection (or re-upload) almost immediately: mean gap of 3.6 µs.
const EAGER: f64 = 1.0e9;

fn secs(base: Instant, s: u64) -> Instant {
    base + Duration::from_secs(s)
}

/// Runs ticks every 3 s from `base` for `seconds`, answering every call successfully.
fn heartbeat_ticks(state: DeviceState, base: Instant, seconds: u64) -> Vec<u64> {
    let mut sent = Vec::new();
    let mut state = state;
    for s in (0..=seconds).step_by(3) {
        let (next, plan) = state.tick(secs(base, s));
        state = next;
        if plan.sysinfo {
            state = state.sysinfo_done(secs(base, s), SysinfoOutcome::Updated);
        }
        if plan.heartbeat.is_some() {
            sent.push(s);
            state = state.heartbeat_done(HeartbeatReply::default());
        }
    }
    sent
}

#[test]
fn first_tick_uploads_sysinfo_then_heartbeats() {
    let base = Instant::now();
    let state = DeviceState::new(base, Timing::default(), QUIET, 1);

    let (_, plan) = state.tick(base);

    assert!(plan.sysinfo);
    assert_eq!(plan.heartbeat, Some(vec![]));
    assert_eq!(plan.audit, None);
}

#[test]
fn heartbeat_goes_out_every_fifteen_seconds_without_connections() {
    let base = Instant::now();
    let state = DeviceState::new(base, Timing::default(), QUIET, 1);

    assert_eq!(heartbeat_ticks(state, base, 45), vec![0, 15, 30, 45]);
}

#[test]
fn uploaded_sysinfo_is_sent_again_only_when_a_heartbeat_asks() {
    // Arrange
    let base = Instant::now();
    let (state, _) = DeviceState::new(base, Timing::default(), QUIET, 1).tick(base);
    let state = state.sysinfo_done(base, SysinfoOutcome::Updated);
    let (state, plan) = state.tick(secs(base, 3));
    assert!(!plan.sysinfo);

    // Act
    let state = state.heartbeat_done(HeartbeatReply {
        sysinfo: true,
        modified_at: None,
    });
    let (_, plan) = state.tick(secs(base, 6));

    // Assert
    assert!(plan.sysinfo);
}

#[test]
fn sysinfo_retries_next_tick_on_id_not_found_and_after_120s_otherwise() {
    let base = Instant::now();
    let (state, _) = DeviceState::new(base, Timing::default(), QUIET, 1).tick(base);

    let state = state.sysinfo_done(base, SysinfoOutcome::IdNotFound);
    let (state, plan) = state.tick(secs(base, 3));
    assert!(plan.sysinfo, "ID_NOT_FOUND: next tick");

    let state = state.sysinfo_done(secs(base, 3), SysinfoOutcome::Other);
    let (state, plan) = state.tick(secs(base, 6));
    assert!(!plan.sysinfo, "other answer: wait");
    let (_, plan) = state.tick(secs(base, 123));
    assert!(plan.sysinfo, "after UPLOAD_SYSINFO_TIMEOUT");
}

#[test]
fn reply_modified_at_is_kept_and_reported() {
    let base = Instant::now();
    let state = DeviceState::new(base, Timing::default(), QUIET, 1);
    assert_eq!(state.modified_at(), 0);

    let state = state.heartbeat_done(HeartbeatReply {
        sysinfo: false,
        modified_at: Some(1_730_000_100),
    });
    assert_eq!(state.modified_at(), 1_730_000_100);
    let state = state.heartbeat_done(HeartbeatReply::default());
    assert_eq!(state.modified_at(), 1_730_000_100, "absent key keeps it");
}

#[test]
fn a_connection_sends_its_three_records_on_consecutive_ticks_and_heartbeats_every_tick() {
    // Arrange
    let base = Instant::now();
    let rates = Rates {
        connections_per_hour: EAGER,
        sysinfo_per_hour: 0.0,
    };
    // The first arrival lies just after `base`, so the connection opens on the second tick.
    let state = DeviceState::new(base, Timing::default(), rates, 7);
    let (state, start) = state.tick(base);
    assert_eq!(start.audit, None);
    let state = state.sysinfo_done(base, SysinfoOutcome::Updated);

    // Act
    let (state, first) = state.tick(secs(base, 3));
    let (state, second) = state.tick(secs(base, 6));
    let (_, third) = state.tick(secs(base, 9));

    // Assert
    let new = first.audit.unwrap();
    assert_eq!((new.step, new.session_id), (AuditStep::New, 0));
    assert_eq!(first.heartbeat, Some(vec![new.conn_id]));
    let authorized = second.audit.unwrap();
    assert_eq!(authorized.step, AuditStep::Authorized);
    assert_eq!(authorized.conn_id, new.conn_id);
    assert_ne!(authorized.session_id, 0);
    assert_eq!(
        second.heartbeat,
        Some(vec![new.conn_id]),
        "live: every tick"
    );
    let close = third.audit.unwrap();
    assert_eq!(close.step, AuditStep::Close);
    assert_eq!(close.session_id, authorized.session_id);
    assert_eq!(close.conn_id, new.conn_id);
}

#[test]
fn successive_connections_get_new_ids() {
    let base = Instant::now();
    let rates = Rates {
        connections_per_hour: EAGER,
        sysinfo_per_hour: 0.0,
    };
    let mut state = DeviceState::new(base, Timing::default(), rates, 7);
    let mut news = Vec::new();
    for s in (0..30).step_by(3) {
        let (next, plan) = state.tick(secs(base, s));
        state = next;
        if let Some(AuditCall {
            step: AuditStep::New,
            conn_id,
            ..
        }) = plan.audit
        {
            news.push(conn_id);
        }
    }

    assert!(news.len() >= 2, "{news:?}");
    assert!(news.windows(2).all(|w| w[1] == w[0] + 1), "{news:?}");
}

#[test]
fn sysinfo_refresh_rate_triggers_re_uploads() {
    let base = Instant::now();
    let rates = Rates {
        connections_per_hour: 0.0,
        sysinfo_per_hour: EAGER,
    };
    let (state, _) = DeviceState::new(base, Timing::default(), rates, 3).tick(base);
    let state = state.sysinfo_done(base, SysinfoOutcome::Updated);

    let (_, plan) = state.tick(secs(base, 3));

    assert!(plan.sysinfo);
}
