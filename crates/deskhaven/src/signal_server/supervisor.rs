//! Startup gate and supervised restarts of the signal server.
//!
//! Every error `SignalServer::run` returns is terminal (bind / rebind failure, UDP self-test,
//! peer registry preload). [`start`] turns a failure of the first attempt into a startup
//! failure (a clear error and exit 1, e.g. for a taken port). Once serving, a failed server is
//! logged and restarted with exponential backoff ([`Backoff`]), re-reading the settings and
//! re-loading the peer registry, until shutdown.

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use deskhaven_config::settings::Snapshot;
use deskhaven_signal::{ControlPlane, PeerStore, SignalConfig, SignalHandle, SignalServer};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{RelaySettings, relay_sync};
use crate::log_safe;

/// Delay before restarting a failed server: `initial`, doubled after each failure up to
/// `max`; back to `initial` after an attempt that ran for at least `max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    pub initial: Duration,
    pub max: Duration,
}

impl Backoff {
    /// 1 s doubling to 60 s.
    pub const DEFAULT: Self = Self {
        initial: Duration::from_secs(1),
        max: Duration::from_secs(60),
    };
}

/// The delay before restarting an attempt that ran for `ran` (launch to failure), given the
/// delay waited before it (`None`: it was the first attempt).
pub(super) fn next_delay(backoff: Backoff, previous: Option<Duration>, ran: Duration) -> Duration {
    match previous {
        Some(previous) if ran < backoff.max => previous.saturating_mul(2).min(backoff.max),
        _ => backoff.initial,
    }
}

/// Builds one server per attempt from fixed parts and the latest settings.
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

    /// Starts a server on the current settings, with a relay sync task that applies every
    /// later change to it.
    fn launch(&mut self, token: &CancellationToken) -> Attempt {
        let relays = RelaySettings::from_snapshot(&self.settings.borrow_and_update());
        let server = SignalServer::new(
            relays.apply_to(&self.base),
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
            relays,
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

/// One running server and its relay sync task.
struct Attempt {
    handle: SignalHandle,
    run: JoinHandle<Result<(), deskhaven_signal::SignalError>>,
    sync: JoinHandle<()>,
    sync_stop: CancellationToken,
    launched: Instant,
}

impl Attempt {
    /// Waits for the server to stop, then stops the relay sync. `Ok` only after a requested
    /// shutdown.
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

/// Whether the current server is serving, across restarts (for `/readyz`).
#[derive(Clone, Debug)]
pub struct SignalStatus(watch::Receiver<SignalHandle>);

impl SignalStatus {
    pub fn is_serving(&self) -> bool {
        self.0.borrow().is_serving()
    }

    /// Waits until the current server (the last one launched) has stopped and drained,
    /// even if its supervisor is gone (a panic drops the supervisor's join handle, not the
    /// server, which then stops on the shutdown token): close the database only after this.
    pub async fn stopped(&self) {
        let current = self.0.borrow().clone();
        current.stopped().await;
    }

    /// The current server's handle.
    #[cfg(test)]
    pub(super) fn handle(&self) -> SignalHandle {
        self.0.borrow().clone()
    }
}

/// A supervised signal server.
pub struct RunningSignal {
    pub status: SignalStatus,
    /// Ends once `token` was cancelled and the current server stopped.
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

/// Starts the server and waits until it serves. A first attempt that fails before serving
/// fails startup; later failures are restarted with `backoff`.
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

/// Supervises from the first attempt on, without the startup gate: a first attempt that
/// fails is retried like any later failure.
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
