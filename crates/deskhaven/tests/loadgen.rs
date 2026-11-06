//! The load generator (M4-04, `cargo xtask loadgen`) against the real binary: a handful of
//! devices for a few seconds on every test backend, with shortened client timers. The 10k
//! run is manual (docs/reference/performance.md).
//! Requires Linux routing all of 127.0.0.0/8 to `lo` (each device has its own address).
#![cfg(unix)]
// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod support;

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use deskhaven_db::testing::{Backend, TestDb, each_backend};
use deskhaven_loadgen::plan::{Rates, Timing};
use deskhaven_loadgen::server::ServerConfig;
use deskhaven_loadgen::stats::{Kind, Phase, Summary};
use deskhaven_loadgen::{Config, Report, run};
use support::{free_port, free_signal_port, workdir};

const DEVICES: usize = 5;
/// The whole run (startup, ~17 s of load, shutdown) must end well within this.
const RUN_LIMIT: Duration = Duration::from_secs(120);
const FLUSH: Duration = Duration::from_secs(5);

fn config(db: &TestDb) -> Config {
    let server = ServerConfig {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_deskhaven")),
        work_dir: workdir(&format!("loadgen-{}", db.backend)),
        database_url: Some(db.url.clone()),
        api_port: free_port(),
        signal_port: free_signal_port(),
        last_seen_flush: FLUSH,
        log_level: "info".to_owned(),
    };
    // Above the server's 2 s heartbeat floor even while a connection is live (every tick).
    let timing = Timing {
        tick: Duration::from_millis(2500),
        heartbeat: Duration::from_secs(5),
        register: Duration::from_secs(2),
        ..Timing::default()
    };
    Config {
        server,
        devices: DEVICES,
        first_ip: Ipv4Addr::new(127, 65, 0, 1),
        timing,
        // A connection about every second per device; a re-upload every two.
        rates: Rates {
            connections_per_hour: 3600.0,
            sysinfo_per_hour: 1800.0,
        },
        idle: Duration::from_secs(1),
        ramp: Duration::from_secs(1),
        settle: Duration::from_secs(4),
        steady: Duration::from_secs(11),
        seed: 7,
        command: "deskhaven tests/loadgen.rs".to_owned(),
    }
}

fn summary(report: &Report, phase: Phase, kind: Kind) -> Summary {
    report
        .summaries
        .iter()
        .find(|(p, k, _)| (*p, *k) == (phase, kind))
        .map(|(_, _, s)| s.clone())
        .unwrap_or_default()
}

fn total(report: &Report, kind: Kind) -> (u64, u64) {
    [Phase::Ramp, Phase::Settle, Phase::Steady]
        .iter()
        .map(|&phase| summary(report, phase, kind))
        .fold((0, 0), |(ok, failed), s| (ok + s.ok, failed + s.failed()))
}

fn assert_client_side(report: &Report) {
    let steady = &report.measured.heartbeat;
    assert!(steady.ok > 0, "no steady heartbeats:\n{report}");
    assert_eq!(steady.failed(), 0, "steady heartbeat failures:\n{report}");
    assert!(steady.p99.is_some());
    let pk = summary(report, Phase::Ramp, Kind::RegisterPk);
    assert_eq!(
        pk.ok,
        u64::try_from(DEVICES).unwrap(),
        "one RegisterPk each:\n{report}"
    );
    assert_eq!(report.measured.devices_failed_to_start, 0, "{report}");
    let (peer_ok, peer_failed) = total(report, Kind::RegisterPeer);
    assert!(peer_ok > 0 && peer_failed == 0, "RegisterPeer:\n{report}");
    let (sysinfo_ok, _) = total(report, Kind::Sysinfo);
    assert!(
        sysinfo_ok >= u64::try_from(DEVICES).unwrap(),
        "sysinfo:\n{report}"
    );
    let (audit_ok, audit_failed) = total(report, Kind::Audit);
    assert!(audit_ok > 0 && audit_failed == 0, "audit:\n{report}");
}

fn assert_server_side(report: &Report, backend: Backend) {
    assert!(
        report.measured.server_exit_clean,
        "both server shutdowns:\n{report}"
    );
    assert!(
        report.measured.cpu_cores.is_some_and(|c| c > 0.0),
        "cpu:\n{report}"
    );
    // Idle RSS of the restart on the populated database, and of the empty one before.
    assert!(
        report.measured.idle_rss_bytes.is_some_and(|b| b > 0),
        "rss after restart:\n{report}"
    );
    assert!(
        report.idle_rss_empty_bytes.is_some_and(|b| b > 0),
        "rss:\n{report}"
    );
    let devices = report.db_run.table("devices");
    assert_eq!(
        devices.inserted,
        i64::try_from(DEVICES).unwrap(),
        "{report}"
    );
    assert!(
        report.db_run.table("connection_sessions").inserted > 0,
        "{report}"
    );
    // At most one row update per device per flush (window 11 s, flush 5 s: 3 + 1). SQLite
    // counts exactly; PostgreSQL statistics may shift a flush across the window's edges.
    let updates = report.measured.device_row_updates;
    let flushes_allowed = match backend {
        Backend::Sqlite => 4,
        Backend::Postgres => 5,
    };
    let bound = i64::try_from(DEVICES * flushes_allowed).unwrap();
    assert!(updates <= bound, "{report}");
    if backend == Backend::Sqlite {
        assert!(
            updates >= i64::try_from(DEVICES).unwrap(),
            "two flushes ran:\n{report}"
        );
    }
    let text = report.to_string();
    assert!(text.contains("## NFR verdicts") && text.contains("| Heartbeat p99 |"));
}

#[tokio::test(flavor = "multi_thread")]
async fn loadgen_drives_devices_and_measures_the_server() {
    each_backend(async |db: &TestDb| {
        // Arrange
        let config = config(db);

        // Act
        let report = tokio::time::timeout(RUN_LIMIT, run(&config))
            .await
            .expect("run timed out")
            .unwrap_or_else(|err| panic!("loadgen failed: {err:?}"));

        // Assert
        assert_client_side(&report);
        assert_server_side(&report, db.backend);
    })
    .await;
}
