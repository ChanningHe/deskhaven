//! The NFR table: one [`Verdict`] per requirement, all of which must pass. Besides the
//! 01-prd §4 thresholds, rows that keep a run from passing while part of the load fails
//! (load reached, registrations, sysinfo / audit, clean shutdowns).

use std::time::Duration;

use super::{Measured, Targets, Verdict, mib};
use crate::stats::Summary;

const NA: &str = "n/a";
/// Rows one audit call writes: its nonce, and the connection row it creates or completes
/// (03 §2.3, M2-15).
const AUDIT_ROWS_PER_CALL: f64 = 2.0;
/// Rows one sysinfo call writes at most: the device row.
const SYSINFO_ROWS_PER_CALL: f64 = 1.0;
/// Transactions one audit or sysinfo call may commit (PostgreSQL, reads included; about one
/// was measured).
const COMMITS_PER_CALL: f64 = 3.0;
/// Rows per last-seen flush statement (`deskhaven_core::devices::FLUSH_BATCH`).
const FLUSH_ROWS_PER_STATEMENT: f64 = 500.0;
/// Commits besides calls and flushes: the generator's own snapshots, background jobs.
const COMMIT_SLACK: f64 = 100.0;

pub fn verdicts(m: &Measured, t: &Targets) -> Vec<Verdict> {
    vec![
        heartbeat_load(m, t),
        register_load(m, t),
        p99(m, t),
        none_failed("Heartbeat failures", &[&m.heartbeat]),
        none_failed("Registration failures", &[&m.register_peer, &m.register_pk]),
        none_failed("Sysinfo and audit failures", &[&m.sysinfo, &m.audit]),
        cpu(m, t),
        idle_rss(m, t),
        steady_rss(m, t),
        batched(m),
        whole_database(m),
        shutdown(m),
        failed_to_start(m),
    ]
}

/// A device whose socket could not be bound never registers; its failure happens at its
/// start (usually in the ramp), so it is counted outside the phase-bound series.
fn failed_to_start(m: &Measured) -> Verdict {
    Verdict {
        nfr: "Devices failed to start",
        target: "0 (any phase)".to_owned(),
        measured: format!("{} of {}", m.devices_failed_to_start, m.devices),
        pass: m.devices_failed_to_start == 0,
    }
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

fn per_second(count: u64, window: Duration) -> f64 {
    match window.as_secs_f64() {
        secs if secs > 0.0 => count as f64 / secs,
        _ => 0.0,
    }
}

/// A rate verdict: `ok` calls in the window against `devices / interval`.
fn rate(nfr: &'static str, m: &Measured, ok: u64, interval: Duration, t: &Targets) -> Verdict {
    let nominal = m.devices as f64 / interval.as_secs_f64().max(f64::MIN_POSITIVE);
    let measured = per_second(ok, m.window);
    Verdict {
        nfr,
        target: format!(
            "≈ {nominal:.0} req/s ({} devices / {} s), at least {:.0}%",
            m.devices,
            interval.as_secs(),
            t.min_rate_ratio * 100.0
        ),
        measured: format!("{measured:.1} req/s"),
        pass: measured >= nominal * t.min_rate_ratio,
    }
}

/// The run is only valid if the server actually saw the nominal heartbeat rate.
fn heartbeat_load(m: &Measured, t: &Targets) -> Verdict {
    rate("Heartbeat load", m, m.heartbeat.ok, m.heartbeat_interval, t)
}

/// Same for the UDP side, so a slow API cannot hide a lower registration rate.
fn register_load(m: &Measured, t: &Targets) -> Verdict {
    rate(
        "RegisterPeer load",
        m,
        m.register_peer.ok,
        m.register_interval,
        t,
    )
}

fn p99(m: &Measured, t: &Targets) -> Verdict {
    Verdict {
        nfr: "Heartbeat p99",
        target: format!("< {}", ms(t.heartbeat_p99)),
        measured: m.heartbeat.p99.map_or_else(|| NA.to_owned(), ms),
        pass: m.heartbeat.p99.is_some_and(|p| p < t.heartbeat_p99),
    }
}

/// No failure at all in the window: on loopback a UDP answer or an HTTP reply is never lost,
/// so any failure is the server's.
fn none_failed(nfr: &'static str, series: &[&Summary]) -> Verdict {
    let failed: u64 = series.iter().map(|s| s.failed()).sum();
    let total: u64 = series.iter().map(|s| s.ok).sum::<u64>() + failed;
    Verdict {
        nfr,
        target: "0".to_owned(),
        measured: format!("{failed} of {total}"),
        pass: failed == 0,
    }
}

fn cpu(m: &Measured, t: &Targets) -> Verdict {
    Verdict {
        nfr: "Server CPU",
        target: format!("< {:.0} core (average over the window)", t.cores),
        measured: m
            .cpu_cores
            .map_or_else(|| NA.to_owned(), |c| format!("{c:.2} cores")),
        pass: m.cpu_cores.is_some_and(|c| c < t.cores),
    }
}

fn idle_rss(m: &Measured, t: &Targets) -> Verdict {
    Verdict {
        nfr: "Idle RSS",
        target: format!(
            "< {} (restarted on the populated database, no load)",
            mib(t.idle_rss_bytes)
        ),
        measured: m.idle_rss_bytes.map_or_else(|| NA.to_owned(), mib),
        pass: m.idle_rss_bytes.is_some_and(|b| b < t.idle_rss_bytes),
    }
}

fn steady_rss(m: &Measured, t: &Targets) -> Verdict {
    Verdict {
        nfr: "Steady RSS",
        target: format!("< {} (largest under load)", mib(t.steady_rss_bytes)),
        measured: m.steady_rss_bytes.map_or_else(|| NA.to_owned(), mib),
        pass: m.steady_rss_bytes.is_some_and(|b| b < t.steady_rss_bytes),
    }
}

/// Last-seen flushes in the window, counting one at either edge; `None` without a flush.
fn flushes(m: &Measured) -> Option<f64> {
    let flush = m.last_seen_flush.as_secs_f64();
    (flush > 0.0).then(|| (m.window.as_secs_f64() / flush).ceil() + 1.0)
}

/// A bound tells batching from per-heartbeat writes only while it stays below the heartbeat
/// count (a flush longer than the heartbeat interval), over at least two flushes.
fn conclusive(m: &Measured, bounds: &[f64]) -> bool {
    let heartbeats = m.heartbeat.ok as f64;
    let flush = m.last_seen_flush.as_secs_f64();
    flush > 0.0
        && m.window.as_secs_f64() >= 2.0 * flush
        && bounds.iter().all(|&bound| bound < heartbeats)
}

const INCONCLUSIVE: &str = "; inconclusive: needs bound < heartbeats and window ≥ 2 flushes";

/// Each flush writes a device's row at most once: at most one `devices` update per device
/// per flush, while writing per heartbeat needs one per heartbeat.
fn batched(m: &Measured) -> Verdict {
    let bound = m.devices as f64 * flushes(m).unwrap_or(f64::INFINITY);
    let (updates, heartbeats) = (m.device_row_updates, m.heartbeat.ok);
    let conclusive = conclusive(m, &[bound]);
    Verdict {
        nfr: "Heartbeat not written per row",
        target: format!(
            "`devices` row updates ≤ devices × (⌈window / flush⌉ + 1) = {bound:.0} \
             (per-heartbeat writes would need ≥ 1 per heartbeat)"
        ),
        measured: format!(
            "{updates} row updates for {heartbeats} heartbeats ({:.2} per heartbeat; \
             window {} s, flush {} s{})",
            updates as f64 / (heartbeats.max(1)) as f64,
            m.window.as_secs(),
            m.last_seen_flush.as_secs(),
            if conclusive { "" } else { INCONCLUSIVE }
        ),
        pass: conclusive && (updates as f64) <= bound,
    }
}

/// The flush bound plus what the audit and sysinfo calls may write, over every table, so a
/// heartbeat that writes any table (or, on PostgreSQL, commits anything) every time fails.
fn whole_database(m: &Measured) -> Verdict {
    let flushes = flushes(m).unwrap_or(f64::INFINITY);
    let (audit, sysinfo) = (calls(&m.audit), calls(&m.sysinfo));
    let rows_bound =
        m.devices as f64 * flushes + AUDIT_ROWS_PER_CALL * audit + SYSINFO_ROWS_PER_CALL * sysinfo;
    let statements = (m.devices as f64 / FLUSH_ROWS_PER_STATEMENT).ceil();
    let commits_bound = statements * flushes + COMMITS_PER_CALL * (audit + sysinfo) + COMMIT_SLACK;
    let bounds: &[f64] = match m.db_commits {
        Some(_) => &[rows_bound, commits_bound],
        None => &[rows_bound],
    };
    let conclusive = conclusive(m, bounds);
    let commits = m.db_commits.map_or_else(String::new, |c| {
        format!("; {c} commits (bound {commits_bound:.0})")
    });
    Verdict {
        nfr: "Database writes, all tables",
        target: "rows ≤ flush bound + 2 per audit call + 1 per sysinfo call; PostgreSQL also \
                 commits ≤ flush statements + 3 per audit / sysinfo call + 100"
            .to_owned(),
        measured: format!(
            "{} rows (bound {rows_bound:.0}){commits} for {} heartbeats{}",
            m.db_row_writes,
            m.heartbeat.ok,
            if conclusive { "" } else { INCONCLUSIVE }
        ),
        pass: conclusive
            && (m.db_row_writes as f64) <= rows_bound
            && m.db_commits.is_none_or(|c| (c as f64) <= commits_bound),
    }
}

fn calls(s: &Summary) -> f64 {
    (s.ok + s.failed()) as f64
}

fn shutdown(m: &Measured) -> Verdict {
    Verdict {
        nfr: "Server shutdown",
        target: "status 0 after SIGTERM (load run and idle restart)".to_owned(),
        measured: match m.server_exit_clean {
            true => "clean".to_owned(),
            false => "not clean".to_owned(),
        },
        pass: m.server_exit_clean,
    }
}
