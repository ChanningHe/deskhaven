//! Turns what a run recorded into its [`Report`].

use std::time::Duration;

use super::{Config, Counters};
use crate::machine::Machine;
use crate::report::{Measured, Report};
use crate::sampler::Usage;
use crate::stats::{Kind, Phase, Recorder};

const PHASES: [Phase; 3] = [Phase::Ramp, Phase::Settle, Phase::Steady];
const KINDS: [Kind; 5] = [
    Kind::Heartbeat,
    Kind::Sysinfo,
    Kind::Audit,
    Kind::RegisterPeer,
    Kind::RegisterPk,
];

/// The report of the load phases; the idle restart fills in its two fields afterwards.
pub(super) fn report(
    config: &Config,
    recorder: &Recorder,
    window: Duration,
    usage: Usage,
    db: &Counters,
) -> Report {
    let db_steady = db.steady_end.since(&db.steady_start);
    let summaries = PHASES
        .iter()
        .flat_map(|&phase| KINDS.iter().map(move |&kind| (phase, kind)))
        .map(|(phase, kind)| (phase, kind, recorder.summary(phase, kind)))
        .filter(|(_, _, s)| s.ok + s.failed() > 0)
        .collect();
    let steady = |kind| recorder.summary(Phase::Steady, kind);
    let measured = Measured {
        devices: config.devices,
        heartbeat_interval: config.timing.heartbeat,
        register_interval: config.timing.register,
        last_seen_flush: config.server.last_seen_flush,
        window,
        heartbeat: steady(Kind::Heartbeat),
        register_peer: steady(Kind::RegisterPeer),
        register_pk: steady(Kind::RegisterPk),
        sysinfo: steady(Kind::Sysinfo),
        audit: steady(Kind::Audit),
        cpu_cores: usage.steady_cores,
        idle_rss_bytes: None,
        device_row_updates: db_steady.table("devices").updated,
        db_row_writes: db_steady.rows_written(),
        db_commits: db_steady.commits,
        server_exit_clean: false,
    };
    Report {
        title: format!(
            "deskhaven load test: {} devices, {}",
            config.devices,
            backend(config)
        ),
        machine: Machine::detect(),
        command: config.command.clone(),
        parameters: parameters(config),
        measured,
        summaries,
        cpu_peak_cores: usage.steady_peak_cores,
        idle_rss_empty_bytes: usage.idle_rss_max,
        steady_rss_max_bytes: usage.steady_rss_max,
        steady_write_bytes: usage.steady_write_bytes,
        db_steady,
        db_run: db.steady_end.since(&db.start),
    }
}

fn backend(config: &Config) -> &'static str {
    match &config.server.database_url {
        Some(url) if url.starts_with("postgres") => "PostgreSQL",
        Some(_) => "SQLite (given URL)",
        None => "SQLite",
    }
}

fn parameters(config: &Config) -> Vec<(String, String)> {
    let d = |d: Duration| humantime::format_duration(d).to_string();
    let t = &config.timing;
    [
        ("Server binary", config.server.bin.display().to_string()),
        ("Database", backend(config).to_owned()),
        ("Devices", config.devices.to_string()),
        (
            "Idle / ramp / settle / steady / idle after restart",
            format!(
                "{} / {} / {} / {} / {}",
                d(config.idle),
                d(config.ramp),
                d(config.settle),
                d(config.steady),
                d(config.idle)
            ),
        ),
        (
            "Client timers: tick / heartbeat / RegisterPeer",
            format!("{} / {} / {}", d(t.tick), d(t.heartbeat), d(t.register)),
        ),
        (
            "`devices.last_seen_flush`",
            d(config.server.last_seen_flush),
        ),
        (
            "Connections (3 audit calls each) per device-hour",
            config.rates.connections_per_hour.to_string(),
        ),
        (
            "Sysinfo re-uploads per device-hour",
            config.rates.sysinfo_per_hour.to_string(),
        ),
        (
            "Device source addresses",
            format!("{} + index", config.first_ip),
        ),
        ("Server log level", config.server.log_level.clone()),
        ("Seed", config.seed.to_string()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}
