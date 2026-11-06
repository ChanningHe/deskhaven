//! A whole run: start the server, sample it idle, start the devices over the ramp, let them
//! settle, measure the steady window, stop everything, report.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::LoadgenError;
use crate::dbstats::{DbProbe, Snapshot};
use crate::device::{self, Identity, Shared};
use crate::machine::Machine;
use crate::plan::{Rates, Timing};
use crate::report::{Measured, Report};
use crate::sampler::{Sampler, Usage};
use crate::server::{Server, ServerConfig};
use crate::stats::{Kind, Phase, Recorder};

/// Startup includes migrations and loading the peer table.
const READY_LIMIT: Duration = Duration::from_secs(60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// `MIN_REG_TIMEOUT` of `rendezvous_mediator.rs`.
const UDP_TIMEOUT: Duration = Duration::from_secs(3);
/// Devices finish their in-flight call (at most [`HTTP_TIMEOUT`]) after the stop.
const DRAIN_LIMIT: Duration = Duration::from_secs(30);
/// `devices.last_seen_flush` bounds (configuration reference).
const MIN_FLUSH: Duration = Duration::from_secs(5);
const MAX_FLUSH: Duration = Duration::from_secs(3600);
const PHASES: [Phase; 3] = [Phase::Ramp, Phase::Settle, Phase::Steady];
const KINDS: [Kind; 5] = [
    Kind::Heartbeat,
    Kind::Sysinfo,
    Kind::Audit,
    Kind::RegisterPeer,
    Kind::RegisterPk,
];

#[derive(Clone, Debug)]
pub struct Config {
    pub server: ServerConfig,
    pub devices: usize,
    /// Source address of device 0; device i uses the i-th address after it.
    pub first_ip: Ipv4Addr,
    pub timing: Timing,
    pub rates: Rates,
    pub idle: Duration,
    pub ramp: Duration,
    pub settle: Duration,
    pub steady: Duration,
    pub seed: u64,
    /// How the run was started, for the report.
    pub command: String,
}

impl Config {
    fn validate(&self) -> Result<(), LoadgenError> {
        let flush = self.server.last_seen_flush;
        let problem = if self.devices == 0 {
            Some("--devices must be at least 1".to_owned())
        } else if flush.subsec_nanos() != 0 || !(MIN_FLUSH..=MAX_FLUSH).contains(&flush) {
            Some("--last-seen-flush must be whole seconds between 5s and 1h".to_owned())
        } else if self.timing.tick.is_zero() || self.timing.register.is_zero() {
            Some("--tick and --register-interval must be positive".to_owned())
        } else if self.timing.heartbeat < self.timing.tick {
            Some("--heartbeat-interval must not be shorter than --tick".to_owned())
        } else if self.steady.is_zero() || self.idle.is_zero() {
            Some("--steady and --idle must be positive".to_owned())
        } else {
            None
        };
        problem.map_or(Ok(()), |p| Err(LoadgenError::Config(p)))
    }
}

/// Runs the load test described by `config`; the server is stopped on every path.
pub async fn run(config: &Config) -> Result<Report, LoadgenError> {
    config.validate()?;
    let identities = (0..config.devices)
        .map(|index| Identity::new(index, config.first_ip))
        .collect::<Result<Vec<_>, _>>()?;
    let mut server = Server::spawn(&config.server)?;
    let measured = measure(config, &mut server, identities).await;
    let stopped = server.stop().await;
    let report = measured?;
    Ok(Report {
        server_exit_clean: stopped?,
        ..report
    })
}

/// Database snapshots at the edges of the run and of the steady window.
struct Counters {
    start: Snapshot,
    steady_start: Snapshot,
    steady_end: Snapshot,
}

async fn measure(
    config: &Config,
    server: &mut Server,
    identities: Vec<Identity>,
) -> Result<Report, LoadgenError> {
    server.wait_ready(READY_LIMIT).await?;
    let probe = DbProbe::open(&server.config().effective_database_url()).await?;
    probe.install().await?;
    let start = probe.snapshot().await?;
    let shared = Arc::new(shared(config));
    let recorder = Arc::clone(&shared.recorder);
    let sampler = Sampler::start(server.pid(), Arc::clone(&recorder));
    tracing::info!(idle = ?config.idle, "sampling the idle server");
    sleep(config.idle).await;
    recorder.set_phase(Phase::Ramp);
    let mut devices = spawn_devices(identities, &shared, config);
    tracing::info!(devices = config.devices, ramp = ?config.ramp, "devices starting");
    sleep(config.ramp).await;
    recorder.set_phase(Phase::Settle);
    sleep(config.settle).await;
    let steady_start = probe.snapshot().await?;
    recorder.set_phase(Phase::Steady);
    tracing::info!(steady = ?config.steady, "steady window");
    let window_started = Instant::now();
    sleep(config.steady).await;
    let window = window_started.elapsed();
    recorder.set_phase(Phase::Drain);
    let steady_end = probe.snapshot().await?;
    shared.stop.cancel();
    drain(&mut devices).await;
    let usage = sampler.finish().await;
    let counters = Counters {
        start,
        steady_start,
        steady_end,
    };
    Ok(report(config, &recorder, window, usage, &counters))
}

fn shared(config: &Config) -> Shared {
    Shared {
        api: config.server.api(),
        signal: config.server.signal(),
        timing: config.timing,
        rates: config.rates,
        recorder: Arc::new(Recorder::default()),
        stop: CancellationToken::new(),
        http_timeout: HTTP_TIMEOUT,
        udp_timeout: UDP_TIMEOUT,
    }
}

/// Device i starts at `ramp × i / n`, so heartbeats are spread evenly once all run.
fn spawn_devices(identities: Vec<Identity>, shared: &Arc<Shared>, config: &Config) -> JoinSet<()> {
    let start = Instant::now();
    let count = identities.len().max(1);
    let mut set = JoinSet::new();
    for identity in identities {
        let offset = config.ramp.mul_f64(identity.index as f64 / count as f64);
        let seed = config.seed ^ (identity.index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        set.spawn(device::run(
            identity,
            Arc::clone(shared),
            start + offset,
            seed,
        ));
    }
    set
}

async fn drain(devices: &mut JoinSet<()>) {
    let joined = timeout(DRAIN_LIMIT, async {
        while let Some(result) = devices.join_next().await {
            if let Err(err) = result {
                tracing::error!(error = %err, "device task failed");
            }
        }
    })
    .await;
    if joined.is_err() {
        tracing::warn!(
            left = devices.len(),
            "devices did not stop in time; aborting them"
        );
        devices.shutdown().await;
    }
}

fn report(
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
    let measured = Measured {
        devices: config.devices,
        heartbeat_interval: config.timing.heartbeat,
        last_seen_flush: config.server.last_seen_flush,
        window,
        heartbeat: recorder.summary(Phase::Steady, Kind::Heartbeat),
        cpu_cores: usage.steady_cores,
        idle_rss_bytes: usage.idle_rss_max,
        device_row_updates: db_steady.table("devices").updated,
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
        steady_rss_max_bytes: usage.steady_rss_max,
        steady_write_bytes: usage.steady_write_bytes,
        db_steady,
        db_run: db.steady_end.since(&db.start),
        // Set by `run` once the server stopped.
        server_exit_clean: false,
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
            "Idle / ramp / settle / steady",
            format!(
                "{} / {} / {} / {}",
                d(config.idle),
                d(config.ramp),
                d(config.settle),
                d(config.steady)
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
