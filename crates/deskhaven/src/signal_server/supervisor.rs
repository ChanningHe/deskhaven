//! A first attempt that fails before serving fails startup (a clear error and exit 1); once
//! serving, a failed server is restarted with exponential backoff until shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use deskhaven_config::settings::Snapshot;
use deskhaven_core::devices::{LiveLookup, LiveRegistrations, LiveUnknown};
use deskhaven_signal::{ControlPlane, PeerStore, SignalConfig, SignalHandle, SignalServer};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{SignalSettings, relay_sync};
use crate::log_safe;

/// Delay before restarting a failed server: `initial`, doubled after each failure up to
/// `max`; back to `initial` after an attempt that ran for at least `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Backoff {
    pub const DEFAULT: Self = Self {
        initial: Duration::from_secs(1),
        max: Duration::from_secs(60),
    };
}

pub(super) fn next_delay(backoff: Backoff, previous: Option<Duration>, ran: Duration) -> Duration {
    match previous {
        Some(previous) if ran < backoff.max => previous.saturating_mul(2).min(backoff.max),
        _ => backoff.initial,
    }
}

pub struct Launcher {
    base: SignalConfig,
    store: Arc<dyn PeerStore>,
    control: Arc<dyn ControlPlane>,
    settings: watch::Receiver<Arc<Snapshot>>,
}

impl Launcher {
    pub fn new(
        base: SignalConfig,
        store: Arc<dyn PeerStore>,
        control: Arc<dyn ControlPlane>,
        settings: watch::Receiver<Arc<Snapshot>>,
    ) -> Self {
        Self {
            base,
            store,
            control,
            settings,
        }
    }

    fn launch(&mut self, token: &CancellationToken) -> Attempt {
        let runtime = SignalSettings::from_snapshot(&self.settings.borrow_and_update());
        let server = SignalServer::new(
            runtime.apply_to(&self.base),
            Arc::clone(&self.store),
            Arc::clone(&self.control),
        );
        let handle = server.handle();
        let run = tokio::spawn(server.run(token.child_token()));
        let sync_stop = token.child_token();
        // The clone has seen the snapshot the server was built from.
        let sync = tokio::spawn(relay_sync::run(
            handle.clone(),
            self.settings.clone(),
            runtime,
            sync_stop.clone(),
        ));
        Attempt {
            handle,
            run,
            sync,
            sync_stop,
            launched: Instant::now(),
        }
    }
}

struct Attempt {
    handle: SignalHandle,
    run: JoinHandle<Result<(), deskhaven_signal::SignalError>>,
    sync: JoinHandle<()>,
    sync_stop: CancellationToken,
    launched: Instant,
}

impl Attempt {
    /// `Ok` only after a requested shutdown.
    async fn finish(self) -> anyhow::Result<()> {
        let res = self.run.await;
        self.sync_stop.cancel();
        if let Err(err) = self.sync.await {
            tracing::warn!(error = %err, "signal relay sync task failed");
        }
        match res {
            Ok(res) => res.map_err(anyhow::Error::from),
            Err(err) => Err(anyhow::Error::new(err).context("signal server task failed")),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SignalStatus(watch::Receiver<SignalHandle>);

pub const LIVE_UUID_TIMEOUT: Duration = Duration::from_secs(2);

impl LiveRegistrations for SignalStatus {
    fn uuid_of<'a>(&'a self, rustdesk_id: &'a str) -> LiveLookup<'a> {
        Box::pin(self.live_uuid(rustdesk_id))
    }
}

impl SignalStatus {
    pub fn is_serving(&self) -> bool {
        self.0.borrow().is_serving()
    }

    /// A server between restarts reloads everything when it starts, so a stopped one is only
    /// logged.
    pub fn reload_peer(&self, id: &str) {
        if let Err(err) = self.0.borrow().reload_peer(id) {
            tracing::debug!(error = %err, "signal server not running; peer reload skipped");
        }
    }

    /// `Ok(None)` also when no server runs: it preloads the database when it starts.
    pub async fn live_uuid(&self, id: &str) -> Result<Option<Vec<u8>>, LiveUnknown> {
        let handle = self.0.borrow().clone();
        match tokio::time::timeout(LIVE_UUID_TIMEOUT, handle.live_uuid(id)).await {
            Ok(Ok(uuid)) => Ok(uuid),
            Ok(Err(err)) => {
                tracing::debug!(error = %err, "signal server not running; no live registration");
                Ok(None)
            }
            Err(_) => {
                tracing::warn!("signal server did not report its live registration in time");
                Err(LiveUnknown)
            }
        }
    }

    /// A panic drops the supervisor's join handle, not the server: close the database only
    /// after this.
    pub async fn stopped(&self) {
        let current = self.0.borrow().clone();
        current.stopped().await;
    }

    pub(super) fn current(&self) -> SignalHandle {
        self.0.borrow().clone()
    }

    #[cfg(test)]
    pub(super) fn handle(&self) -> SignalHandle {
        self.current()
    }
}

pub struct RunningSignal {
    pub status: SignalStatus,
    pub task: JoinHandle<()>,
}

impl RunningSignal {
    fn spawn(
        launcher: Launcher,
        attempt: Attempt,
        backoff: Backoff,
        token: &CancellationToken,
    ) -> Self {
        let (status, status_rx) = watch::channel(attempt.handle.clone());
        let task = tokio::spawn(supervise(launcher, attempt, status, backoff, token.clone()));
        Self {
            status: SignalStatus(status_rx),
            task,
        }
    }
}

pub async fn start(
    mut launcher: Launcher,
    backoff: Backoff,
    token: &CancellationToken,
) -> anyhow::Result<RunningSignal> {
    let attempt = launcher.launch(token);
    if attempt.handle.serving().await.is_err() {
        let err = match attempt.finish().await {
            Ok(()) => anyhow!("stopped before serving"),
            Err(err) => err,
        };
        return Err(err.context("signal server failed to start"));
    }
    tracing::info!("signal server serving");
    Ok(RunningSignal::spawn(launcher, attempt, backoff, token))
}

pub fn spawn_supervised(
    mut launcher: Launcher,
    backoff: Backoff,
    token: &CancellationToken,
) -> RunningSignal {
    let attempt = launcher.launch(token);
    RunningSignal::spawn(launcher, attempt, backoff, token)
}

async fn supervise(
    mut launcher: Launcher,
    first: Attempt,
    status: watch::Sender<SignalHandle>,
    backoff: Backoff,
    token: CancellationToken,
) {
    let mut attempt = first;
    let mut previous = None;
    loop {
        let launched = attempt.launched;
        let res = attempt.finish().await;
        let ran = launched.elapsed();
        if token.is_cancelled() {
            if let Err(err) = res {
                tracing::warn!(error = %log_safe::render(&err), "signal server stopped with an error");
            }
            return;
        }
        let err = res.err().unwrap_or_else(|| anyhow!("stopped unexpectedly"));
        let delay = next_delay(backoff, previous, ran);
        tracing::error!(
            error = %log_safe::render(&err),
            retry_in = ?delay,
            "signal server failed; restarting"
        );
        tokio::select! {
            () = token.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        previous = Some(delay);
        attempt = launcher.launch(&token);
        status.send_replace(attempt.handle.clone());
    }
}
