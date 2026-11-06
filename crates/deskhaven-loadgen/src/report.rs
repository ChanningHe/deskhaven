//! The run's results and the NFR verdicts (01-prd §4), rendered as Markdown ([`render`]).

mod render;
mod verdicts;

pub use verdicts::verdicts;

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
    /// RSS under load (the steady window); 01-prd §4 sets it for 10k devices.
    pub steady_rss_bytes: u64,
    /// Share of the nominal heartbeat rate (devices / interval) the run must reach.
    pub min_rate_ratio: f64,
}

pub const NFR: Targets = Targets {
    heartbeat_p99: Duration::from_millis(50),
    cores: 1.0,
    idle_rss_bytes: 64 * 1024 * 1024,
    steady_rss_bytes: 256 * 1024 * 1024,
    min_rate_ratio: 0.95,
};

/// What the verdicts are computed from (steady window unless noted).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Measured {
    pub devices: usize,
    pub heartbeat_interval: Duration,
    /// RegisterPeer period of the simulated clients.
    pub register_interval: Duration,
    pub last_seen_flush: Duration,
    /// Actual length of the steady window.
    pub window: Duration,
    pub heartbeat: Summary,
    pub register_peer: Summary,
    pub register_pk: Summary,
    pub sysinfo: Summary,
    pub audit: Summary,
    pub cpu_cores: Option<f64>,
    /// Largest RSS of the server restarted on the populated database, without load.
    pub idle_rss_bytes: Option<u64>,
    /// Largest RSS in the steady window, under load.
    pub steady_rss_bytes: Option<u64>,
    /// Rows of `devices` updated in the window.
    pub device_row_updates: i64,
    /// Rows written to any table in the window.
    pub db_row_writes: i64,
    /// PostgreSQL: transactions committed in the window (reads included).
    pub db_commits: Option<i64>,
    /// Both server processes (load run, idle restart) exited with status 0 after SIGTERM.
    pub server_exit_clean: bool,
    /// Devices whose setup failed (socket bind), in any phase: they never registered.
    pub devices_failed_to_start: u64,
}

/// One row of the NFR table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub nfr: &'static str,
    pub target: String,
    pub measured: String,
    pub pass: bool,
}

pub(crate) fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
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
    /// Largest RSS in the idle phase before the load, on the empty database (information).
    pub idle_rss_empty_bytes: Option<u64>,
    pub steady_write_bytes: Option<u64>,
    pub db_steady: Snapshot,
    pub db_run: Snapshot,
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
