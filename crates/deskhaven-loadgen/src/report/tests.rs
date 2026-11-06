use std::collections::BTreeMap;

use super::*;
use crate::stats::Failure;

/// 10k devices, 180 s window, 60 s flush: a passing run.
fn passing() -> Measured {
    Measured {
        devices: 10_000,
        heartbeat_interval: Duration::from_secs(15),
        last_seen_flush: Duration::from_secs(60),
        window: Duration::from_secs(180),
        heartbeat: Summary {
            ok: 120_000,
            p99: Some(Duration::from_millis(4)),
            ..Summary::default()
        },
        cpu_cores: Some(0.4),
        idle_rss_bytes: Some(20 * 1024 * 1024),
        device_row_updates: 30_000,
    }
}

fn verdict(m: &Measured, nfr: &str) -> Verdict {
    verdicts(m, &NFR)
        .into_iter()
        .find(|v| v.nfr == nfr)
        .unwrap_or_else(|| panic!("no verdict {nfr}"))
}

const RATE: &str = "Heartbeat load";
const P99: &str = "Heartbeat p99";
const FAILURES: &str = "Heartbeat failures";
const CPU: &str = "Server CPU";
const RSS: &str = "Idle RSS";
const BATCHED: &str = "Heartbeat not written per row";

#[test]
fn a_good_run_passes_every_nfr() {
    let all = verdicts(&passing(), &NFR);

    assert_eq!(all.len(), 6);
    assert!(all.iter().all(|v| v.pass), "{all:#?}");
    assert_eq!(verdict(&passing(), RATE).measured, "666.7 req/s");
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

    let failing = Measured {
        heartbeat: Summary {
            failures: BTreeMap::from([(Failure::Status(429), 1)]),
            ..passing().heartbeat
        },
        ..passing()
    };
    assert!(!verdict(&failing, FAILURES).pass);
    assert!(verdict(&passing(), FAILURES).pass);
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
