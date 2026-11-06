//! A whole run: start the server, sample it idle, start the devices over the ramp, let them
//! settle, measure the steady window, stop everything; then restart the server on the now
//! populated database and sample its idle RSS. SIGINT / SIGTERM stop the server too.

mod interrupt;
mod summary;

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::{Instant, sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::LoadgenError;
use crate::dbstats::{DbProbe, Snapshot};
use crate::device::{self, Identity, Shared};
use crate::fdlimit;
use crate::plan::{Rates, Timing};
use crate::report::{Measured, Report};
use crate::sampler::{self, Sampler};
use crate::server::{Server, ServerConfig};
use crate::stats::{Phase, Recorder};
use interrupt::Interrupt;

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
/// Spreads per-device seeds.
const SEED_MIX: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Debug)]
pub struct Config {
    pub server: ServerConfig,
    pub devices: usize,
    /// Source address of device 0; device i uses the i-th address after it.
    pub first_ip: Ipv4Addr,
    pub timing: Timing,
    pub rates: Rates,
    /// Length of both idle samplings: before the load, and after the restart.
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

/// Runs the load test described by `config`; every server it starts is stopped on every path.
pub async fn run(config: &Config) -> Result<Report, LoadgenError> {
    config.validate()?;
    fdlimit::ensure(config.devices)?;
    let identities = (0..config.devices)
        .map(|index| Identity::new(index, config.first_ip))
        .collect::<Result<Vec<_>, _>>()?;
    // Dropping `_watcher` (end of the run) stops watching the signals.
    let (interrupt, _watcher) = Interrupt::watch();
    let mut server = Server::spawn(&config.server)?;
    let server_config = server.config().clone();
    let measured = interrupt
        .guard(measure(config, &mut server, identities))
        .await;
    let first_clean = server.stop().await;
    let report = measured?;
    let first_clean = first_clean?;
    // A signal during that stop skips the restart.
    interrupt.check()?;
    let (idle_rss, second_clean) = idle_after_load(&server_config, config.idle, &interrupt).await?;
    let measured = Measured {
        idle_rss_bytes: Some(idle_rss),
        server_exit_clean: first_clean && second_clean,
        ..report.measured
    };
    Ok(Report { measured, ..report })
}

/// Restarts the server on the database the load left behind (every device registered) and
/// takes the largest RSS over `window` without load. Returns it and whether it stopped cleanly.
async fn idle_after_load(
    config: &ServerConfig,
    window: Duration,
    interrupt: &Interrupt,
) -> Result<(u64, bool), LoadgenError> {
    let mut server = Server::restart(config)?;
    let pid = server.pid();
    let sampled = interrupt
        .guard(async {
            server.wait_ready(READY_LIMIT).await?;
            tracing::info!(
                ?window,
                "sampling the idle server on the populated database"
            );
            sampler::max_rss(pid, window).await
        })
        .await;
    let clean = server.stop().await;
    Ok((sampled?, clean?))
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
    Ok(summary::report(config, &recorder, window, usage, &counters))
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

/// Device i starts at `ramp × i / n`, so its calls are spread evenly once all run. Its
/// registration channel and its client API timer are separate tasks, as in the client
/// (`RendezvousMediator` and `start_hbbs_sync`), so a slow API never delays a RegisterPeer.
fn spawn_devices(identities: Vec<Identity>, shared: &Arc<Shared>, config: &Config) -> JoinSet<()> {
    let start = Instant::now();
    let count = identities.len().max(1);
    let mut set = JoinSet::new();
    for identity in identities {
        let offset = config.ramp.mul_f64(identity.index as f64 / count as f64);
        let seed = config.seed ^ (identity.index as u64).wrapping_mul(SEED_MIX);
        let at = start + offset;
        set.spawn(device::run_api(
            identity.clone(),
            Arc::clone(shared),
            at,
            seed,
        ));
        set.spawn(device::run_signal(identity, Arc::clone(shared), at));
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
