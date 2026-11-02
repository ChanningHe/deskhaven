//! `deskhaven serve`: configuration, logging, startup, serving until
//! SIGTERM/SIGINT, then a bounded shutdown (02 §7).
//!
//! Exit codes: 0 after a clean shutdown (also when a signal arrives during
//! startup), 1 on any startup failure (including a signal server that cannot
//! serve, e.g. a taken port) or when shutdown overruns its deadline.

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use deskhaven_config::bootstrap::{Bootstrap, LoadOptions, load};
use deskhaven_core::state::AppState;
use deskhaven_http::ServeError;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::signal_server::SignalStatus;
use crate::startup::{self, Prepared, Started};
use crate::{http, log_safe, logging, shutdown};

/// Hard bound from the signal to process exit (02 §7), split into
/// [`DRAIN_TIMEOUT`] + [`DB_CLOSE_TIMEOUT`] + [`RUNTIME_SHUTDOWN_GRACE`].
/// The signal server stops during the drain share, in parallel with HTTP
/// (its own write-behind flush is bounded by 5 s), and before the database
/// closes, so its last registrations still reach the store.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
/// Blocking tasks still running when the runtime is dropped get this long.
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// Closing the database pool.
const DB_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Inside the runtime: HTTP drain plus database close.
const IN_RUNTIME_DEADLINE: Duration = SHUTDOWN_DEADLINE.saturating_sub(RUNTIME_SHUTDOWN_GRACE);
/// HTTP drain share of the deadline.
pub const DRAIN_TIMEOUT: Duration = IN_RUNTIME_DEADLINE.saturating_sub(DB_CLOSE_TIMEOUT);

pub fn run(options: &LoadOptions) -> ExitCode {
    let loaded = match load(options).context("failed to load configuration") {
        Ok(loaded) => loaded,
        Err(err) => return report_before_logging(&err),
    };
    if let Err(err) = logging::init(&loaded.config.log) {
        return report_before_logging(&err);
    }
    for warning in &loaded.warnings {
        tracing::warn!("{warning}");
    }
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        config_file = ?loaded.config_file,
        data_dir = %loaded.config.data_dir.display(),
        "deskhaven starting"
    );
    boot(&loaded.config).unwrap_or_else(|err| {
        tracing::error!(error = %log_safe::render(&err), "startup failed");
        ExitCode::FAILURE
    })
}

fn report_before_logging(err: &anyhow::Error) -> ExitCode {
    // Nothing sensible remains if stderr itself is gone.
    let _ = writeln!(std::io::stderr().lock(), "error: {}", log_safe::render(err));
    ExitCode::FAILURE
}

fn boot(config: &Bootstrap) -> anyhow::Result<ExitCode> {
    let prepared = startup::prepare(config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    let result = runtime.block_on(serve(config, prepared));
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
    result
}

async fn serve(config: &Bootstrap, prepared: Prepared) -> anyhow::Result<ExitCode> {
    let token = CancellationToken::new();
    shutdown::spawn_signal_listener(token.clone())?;
    let started = tokio::select! {
        // A cancelled startup may also fail (the signal server stops before
        // serving); the request wins.
        biased;
        () = token.cancelled() => {
            tracing::info!("shutdown requested before startup completed");
            return Ok(ExitCode::SUCCESS);
        }
        started = startup::start(config, prepared, DRAIN_TIMEOUT, &token) => started?,
    };
    serve_until_shutdown(started, token).await
}

async fn serve_until_shutdown(
    started: Started,
    token: CancellationToken,
) -> anyhow::Result<ExitCode> {
    let Started {
        state,
        listeners,
        http: http_config,
        admin_routes,
        signal,
        lock: _lock,
    } = started;
    let (signal_status, mut signal_task) = signal.map(|s| (s.status, s.task)).unzip();
    let apps = http::apps(
        Arc::clone(&state),
        admin_routes,
        &token,
        signal_status.clone(),
    );
    let signal_status = signal_status.as_ref();
    let mut serving = tokio::spawn({
        let token = token.clone();
        async move { listeners.serve(apps, &http_config, token).await }
    });
    tracing::info!("ready");

    tokio::select! {
        joined = &mut serving => {
            token.cancel();
            let stop = stop_signal(signal_task, signal_status);
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, stop).await;
            close_db(&state).await;
            return Err(stopped_early(joined));
        }
        joined = signal_ended(&mut signal_task) => {
            // The supervisor only returns after cancellation: it panicked. Its server is
            // detached, not gone: wait for it to drain before closing the database.
            token.cancel();
            let stop = async { tokio::join!(serving, stop_signal(None, signal_status)) };
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, stop).await;
            close_db(&state).await;
            return Err(match joined {
                Ok(()) => anyhow!("signal server supervisor stopped unexpectedly"),
                Err(err) => anyhow::Error::new(err).context("signal server supervisor failed"),
            });
        }
        () = token.cancelled() => {}
    }

    let finish = async {
        let (joined, ()) = tokio::join!(serving, stop_signal(signal_task, signal_status));
        close_db(&state).await;
        joined
    };
    match tokio::time::timeout(IN_RUNTIME_DEADLINE, finish).await {
        Ok(joined) => {
            joined.context("HTTP server task failed")??;
            tracing::info!("shutdown complete");
            Ok(ExitCode::SUCCESS)
        }
        Err(_) => {
            tracing::error!(deadline = ?SHUTDOWN_DEADLINE, "shutdown deadline exceeded; exiting");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// The supervisor task's end; never with the signal server disabled.
async fn signal_ended(task: &mut Option<JoinHandle<()>>) -> Result<(), JoinError> {
    match task {
        Some(task) => task.await,
        None => std::future::pending().await,
    }
}

/// Waits for the supervisor and for the current signal server to stop and drain; the latter
/// also when the supervisor is gone (it panicked), so the database never closes while a
/// registration writer may still flush.
async fn stop_signal(task: Option<JoinHandle<()>>, status: Option<&SignalStatus>) {
    if let Some(task) = task
        && let Err(err) = task.await
    {
        tracing::warn!(error = %err, "signal server supervisor failed");
    }
    if let Some(status) = status {
        status.stopped().await;
    }
}

/// The server only returns after shutdown was requested; anything else is a fault.
fn stopped_early(joined: Result<Result<(), ServeError>, JoinError>) -> anyhow::Error {
    match joined {
        Ok(Ok(())) => anyhow!("HTTP server stopped unexpectedly"),
        Ok(Err(err)) => err.into(),
        Err(err) => anyhow::Error::new(err).context("HTTP server task failed"),
    }
}

/// Closes the pool within [`DB_CLOSE_TIMEOUT`] (both the shutdown and the
/// early-stop path); failures are logged only, the process is exiting.
async fn close_db(state: &AppState) {
    match tokio::time::timeout(DB_CLOSE_TIMEOUT, state.db.close_by_ref()).await {
        Ok(Ok(())) => tracing::info!("database closed"),
        Ok(Err(err)) => {
            tracing::warn!(error = %log_safe::render_error(&err), "closing the database failed")
        }
        Err(_) => tracing::warn!(timeout = ?DB_CLOSE_TIMEOUT, "closing the database timed out"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_budget_adds_up_to_the_deadline() {
        assert_eq!(
            DRAIN_TIMEOUT + DB_CLOSE_TIMEOUT + RUNTIME_SHUTDOWN_GRACE,
            SHUTDOWN_DEADLINE
        );
        assert!(!DRAIN_TIMEOUT.is_zero());
    }
}
