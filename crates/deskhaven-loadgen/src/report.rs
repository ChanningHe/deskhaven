//! The run's results and the NFR verdicts (01-prd §4), rendered as Markdown ([`render`]).

mod render;

use std::time::Duration;

use crate::dbstats::Snapshot;
use crate::machine::Machine;
use crate::stats::{Kind, Phase, Summary};

/// 01-prd §4 thresholds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Targets {
    pub heartbeat_p99: Duration,
    pub cores: f64,
    pub idle_rss_bytes: u64,
    /// Share of the nominal heartbeat rate (devices / interval) the run must reach.
    pub min_rate_ratio: f64,
}

pub const NFR: Targets = Targets {
    heartbeat_p99: Duration::from_millis(50),
    cores: 1.0,
    idle_rss_bytes: 64 * 1024 * 1024,
    min_rate_ratio: 0.95,
};

/// What the verdicts are computed from (steady window unless noted).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    pub devices: usize,
    pub heartbeat_interval: Duration,
    pub last_seen_flush: Duration,
    /// Actual length of the steady window.
    pub window: Duration,
    pub heartbeat: Summary,
    pub cpu_cores: Option<f64>,
    /// Largest RSS in the idle phase.
    pub idle_rss_bytes: Option<u64>,
    /// Rows of `devices` updated in the window.
    pub device_row_updates: i64,
}

/// One row of the NFR table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub nfr: &'static str,
    pub target: String,
    pub measured: String,
    pub pass: bool,
}

pub fn verdicts(m: &Measured, t: &Targets) -> Vec<Verdict> {
    vec![
        load(m, t),
        p99(m, t),
        failures(m),
        cpu(m, t),
        idle_rss(m, t),
        batched(m),
    ]
}

const NA: &str = "n/a";

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1000.0)
}

pub(crate) fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// The run is only valid if the server actually saw the nominal heartbeat rate.
fn load(m: &Measured, t: &Targets) -> Verdict {
    let nominal = m.devices as f64 / m.heartbeat_interval.as_secs_f64().max(f64::MIN_POSITIVE);
    let window = m.window.as_secs_f64();
    let rate = match window > 0.0 {
        true => m.heartbeat.ok as f64 / window,
        false => 0.0,
    };
    Verdict {
        nfr: "Heartbeat load",
        target: format!(
            "≈ {nominal:.0} req/s ({} devices / {} s), at least {:.0}%",
            m.devices,
            m.heartbeat_interval.as_secs(),
            t.min_rate_ratio * 100.0
        ),
        measured: format!("{rate:.1} req/s"),
        pass: rate >= nominal * t.min_rate_ratio,
    }
}

fn p99(m: &Measured, t: &Targets) -> Verdict {
    Verdict {
        nfr: "Heartbeat p99",
        target: format!("< {}", ms(t.heartbeat_p99)),
        measured: m.heartbeat.p99.map_or_else(|| NA.to_owned(), ms),
        pass: m.heartbeat.p99.is_some_and(|p| p < t.heartbeat_p99),
    }
}

fn failures(m: &Measured) -> Verdict {
    let failed = m.heartbeat.failed();
    Verdict {
        nfr: "Heartbeat failures",
        target: "0".to_owned(),
        measured: format!("{failed} of {}", m.heartbeat.ok + failed),
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
        target: format!("< {}", mib(t.idle_rss_bytes)),
        measured: m.idle_rss_bytes.map_or_else(|| NA.to_owned(), mib),
        pass: m.idle_rss_bytes.is_some_and(|b| b < t.idle_rss_bytes),
    }
}

/// Each flush writes a device's row at most once, so the window allows at most one write per
/// device per flush interval (plus one for a flush at either edge). Writing per heartbeat
/// needs one per heartbeat, so the check is conclusive only while that bound is below the
/// heartbeat count (a flush longer than the heartbeat interval), over at least two flushes.
fn batched(m: &Measured) -> Verdict {
    let flush = m.last_seen_flush.as_secs_f64();
    let flushes = match flush > 0.0 {
        true => (m.window.as_secs_f64() / flush).ceil(),
        false => f64::INFINITY,
    };
    let bound = m.devices as f64 * (flushes + 1.0);
    let (updates, heartbeats) = (m.device_row_updates, m.heartbeat.ok);
    let conclusive = bound < heartbeats as f64 && m.window.as_secs_f64() >= 2.0 * flush;
    let note = match conclusive {
        true => "",
        false => "; inconclusive: needs bound < heartbeats and window ≥ 2 flushes",
    };
    Verdict {
        nfr: "Heartbeat not written per row",
        target: format!(
            "`devices` row updates ≤ devices × (⌈window / flush⌉ + 1) = {bound:.0} \
             (per-heartbeat writes would need ≥ 1 per heartbeat)"
        ),
        measured: format!(
            "{updates} row updates for {heartbeats} heartbeats ({:.2} per heartbeat; \
             window {} s, flush {} s{note})",
            updates as f64 / (heartbeats.max(1)) as f64,
            m.window.as_secs(),
            m.last_seen_flush.as_secs()
        ),
        pass: conclusive && (updates as f64) <= bound,
    }
}

/// Everything the report shows.
#[derive(Clone, Debug)]
pub struct Report {
    pub title: String,
    pub machine: Machine,
    pub command: String,
    pub parameters: Vec<(String, String)>,
    pub measured: Measured,
    pub summaries: Vec<(Phase, Kind, Summary)>,
    pub cpu_peak_cores: Option<f64>,
    pub steady_rss_max_bytes: Option<u64>,
    pub steady_write_bytes: Option<u64>,
    pub db_steady: Snapshot,
    pub db_run: Snapshot,
    pub server_exit_clean: bool,
}

impl Report {
    pub fn verdicts(&self) -> Vec<Verdict> {
        verdicts(&self.measured, &NFR)
    }

    pub fn passed(&self) -> bool {
        self.verdicts().iter().all(|v| v.pass)
    }
}

#[cfg(test)]
mod tests;
