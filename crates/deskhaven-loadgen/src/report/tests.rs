use std::collections::BTreeMap;

use super::*;
use crate::stats::Failure;

fn ok(count: u64) -> Summary {
    Summary {
        ok: count,
        ..Summary::default()
    }
}

fn failing(kind: Failure) -> Summary {
    Summary {
        failures: BTreeMap::from([(kind, 1)]),
        ..ok(10)
    }
}

/// 10k devices, 180 s window, 60 s flush: a passing run (the measured 10k figures).
fn passing() -> Measured {
    Measured {
        devices: 10_000,
        heartbeat_interval: Duration::from_secs(15),
        register_interval: Duration::from_secs(12),
        last_seen_flush: Duration::from_secs(60),
        window: Duration::from_secs(180),
        heartbeat: Summary {
            p99: Some(Duration::from_millis(4)),
            ..ok(120_000)
        },
        register_peer: ok(150_000),
        register_pk: Summary::default(),
        sysinfo: ok(474),
        audit: ok(1_520),
        cpu_cores: Some(0.4),
        idle_rss_bytes: Some(20 * 1024 * 1024),
        device_row_updates: 30_000,
        db_row_writes: 33_040,
        db_commits: Some(2_093),
        server_exit_clean: true,
        devices_failed_to_start: 0,
    }
}

fn verdict(m: &Measured, nfr: &str) -> Verdict {
    verdicts(m, &NFR)
        .into_iter()
        .find(|v| v.nfr == nfr)
        .unwrap_or_else(|| panic!("no verdict {nfr}"))
}

const RATE: &str = "Heartbeat load";
const REGISTER_RATE: &str = "RegisterPeer load";
const P99: &str = "Heartbeat p99";
const FAILURES: &str = "Heartbeat failures";
const SIGNAL_FAILURES: &str = "Registration failures";
const CALL_FAILURES: &str = "Sysinfo and audit failures";
const CPU: &str = "Server CPU";
const RSS: &str = "Idle RSS";
const BATCHED: &str = "Heartbeat not written per row";
const WHOLE_DB: &str = "Database writes, all tables";
const SHUTDOWN: &str = "Server shutdown";
const START: &str = "Devices failed to start";

#[test]
fn a_good_run_passes_every_nfr() {
    let all = verdicts(&passing(), &NFR);

    assert_eq!(all.len(), 12);
    assert!(all.iter().all(|v| v.pass), "{all:#?}");
    assert_eq!(verdict(&passing(), RATE).measured, "666.7 req/s");
    assert_eq!(verdict(&passing(), REGISTER_RATE).measured, "833.3 req/s");
    assert_eq!(verdict(&passing(), P99).measured, "4.00 ms");
    assert_eq!(verdict(&passing(), CPU).measured, "0.40 cores");
    assert_eq!(verdict(&passing(), RSS).measured, "20.0 MiB");
}

#[test]
fn each_threshold_fails_on_its_own() {
    let slow = Measured {
        heartbeat: Summary {
            p99: Some(Duration::from_millis(50)),
            ..passing().heartbeat
        },
        ..passing()
    };
    assert!(!verdict(&slow, P99).pass, "p99 must be below 50 ms");

    let busy = Measured {
        cpu_cores: Some(1.0),
        ..passing()
    };
    assert!(!verdict(&busy, CPU).pass);

    let fat = Measured {
        idle_rss_bytes: Some(64 * 1024 * 1024),
        ..passing()
    };
    assert!(!verdict(&fat, RSS).pass);

    let failing_beats = Measured {
        heartbeat: Summary {
            failures: BTreeMap::from([(Failure::Status(429), 1)]),
            ..passing().heartbeat
        },
        ..passing()
    };
    assert!(!verdict(&failing_beats, FAILURES).pass);
    assert!(verdict(&passing(), FAILURES).pass);
}

#[test]
fn signal_registration_failures_fail_the_run() {
    let peer_timeouts = Measured {
        register_peer: failing(Failure::Timeout),
        ..passing()
    };
    let pk_rejected = Measured {
        register_pk: failing(Failure::Body("UUID_MISMATCH".to_owned())),
        ..passing()
    };

    assert!(!verdict(&peer_timeouts, SIGNAL_FAILURES).pass);
    assert!(!verdict(&pk_rejected, SIGNAL_FAILURES).pass);
    assert!(verdict(&passing(), SIGNAL_FAILURES).pass);
}

#[test]
fn sysinfo_or_audit_failures_fail_the_run() {
    let sysinfo = Measured {
        sysinfo: failing(Failure::Status(429)),
        ..passing()
    };
    let audit = Measured {
        audit: failing(Failure::Status(500)),
        ..passing()
    };

    assert!(!verdict(&sysinfo, CALL_FAILURES).pass);
    assert!(!verdict(&audit, CALL_FAILURES).pass);
    assert!(verdict(&passing(), CALL_FAILURES).pass);
}

#[test]
fn an_unclean_server_shutdown_fails_the_run() {
    let unclean = Measured {
        server_exit_clean: false,
        ..passing()
    };

    assert!(!verdict(&unclean, SHUTDOWN).pass);
    assert!(verdicts(&unclean, &NFR).iter().any(|v| !v.pass));
}

#[test]
fn a_device_that_never_started_fails_the_run_whatever_the_phase() {
    // A bind failure happens at the device's start, in the ramp, outside the steady window.
    let lost = Measured {
        devices_failed_to_start: 1,
        ..passing()
    };

    let v = verdict(&lost, START);

    assert!(!v.pass);
    assert_eq!(v.measured, "1 of 10000");
    assert!(verdict(&passing(), START).pass);
}

#[test]
fn too_little_load_invalidates_the_run() {
    let light = Measured {
        heartbeat: Summary {
            ok: 100_000,
            ..passing().heartbeat
        },
        ..passing()
    };
    assert!(!verdict(&light, RATE).pass, "555 req/s < 95% of 667");

    // A slow API must not hide a lower UDP rate: 600 req/s < 95% of 833.
    let quiet_signal = Measured {
        register_peer: ok(108_000),
        ..passing()
    };
    assert!(!verdict(&quiet_signal, REGISTER_RATE).pass);
}

#[test]
fn per_row_heartbeat_writes_fail_the_batching_nfr() {
    // Bound: 10k × (180 / 60 + 1) = 40k row updates.
    let per_row = Measured {
        device_row_updates: 120_000,
        ..passing()
    };
    assert!(!verdict(&per_row, BATCHED).pass);
    let at_bound = Measured {
        device_row_updates: 40_000,
        ..passing()
    };
    assert!(verdict(&at_bound, BATCHED).pass);
    let short = Measured {
        window: Duration::from_secs(100),
        ..passing()
    };
    assert!(!verdict(&short, BATCHED).pass, "needs two flush intervals");
}

#[test]
fn batching_check_is_inconclusive_when_the_bound_reaches_the_heartbeat_count() {
    // Flush 10 s < heartbeat 15 s: every device is written at every flush (RegisterPeer
    // refreshes presence too), more rows than heartbeats and still batched. The bound
    // (10k × 19) is above the heartbeat count, so the window cannot tell batching from
    // per-heartbeat writes.
    let fast_flush = Measured {
        last_seen_flush: Duration::from_secs(10),
        device_row_updates: 180_000,
        ..passing()
    };

    let v = verdict(&fast_flush, BATCHED);

    assert!(!v.pass);
    assert!(v.measured.contains("inconclusive"), "{}", v.measured);
}

#[test]
fn a_heartbeat_writing_any_other_table_fails_the_whole_database_bound() {
    // Bound: 40k (devices) + 2 × 1520 (audit) + 1 × 474 (sysinfo) rows.
    let other_table = Measured {
        db_row_writes: 33_040 + 120_000,
        ..passing()
    };
    assert!(!verdict(&other_table, WHOLE_DB).pass);

    // A transaction per heartbeat (PostgreSQL) fails even without row writes.
    let commit_per_beat = Measured {
        db_commits: Some(122_093),
        ..passing()
    };
    assert!(!verdict(&commit_per_beat, WHOLE_DB).pass);

    // SQLite has no transaction counter: rows alone decide.
    let sqlite = Measured {
        db_commits: None,
        ..passing()
    };
    assert!(verdict(&sqlite, WHOLE_DB).pass);
}

#[test]
fn whole_database_bound_is_inconclusive_when_it_reaches_the_heartbeat_count() {
    let few_beats = Measured {
        heartbeat: Summary {
            ok: 40_000,
            ..passing().heartbeat
        },
        ..passing()
    };

    let v = verdict(&few_beats, WHOLE_DB);

    assert!(!v.pass);
    assert!(v.measured.contains("inconclusive"), "{}", v.measured);
}

#[test]
fn missing_measurements_fail() {
    let blind = Measured {
        heartbeat: Summary::default(),
        cpu_cores: None,
        idle_rss_bytes: None,
        ..passing()
    };

    assert!(!verdict(&blind, P99).pass);
    assert!(!verdict(&blind, CPU).pass);
    assert!(!verdict(&blind, RSS).pass);
    assert_eq!(verdict(&blind, CPU).measured, "n/a");
}
