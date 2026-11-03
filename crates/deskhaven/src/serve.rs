//! `deskhaven serve`: configuration, logging, startup, serving until
//! SIGTERM/SIGINT, then a bounded shutdown (02 §7).
//!
//! Exit codes: 0 after a clean shutdown (also when a signal arrives during
//! startup), 1 on any startup failure (including a signal server that cannot
//! serve, e.g. a taken port) or when shutdown overruns its deadline.

use std::future::Future;
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

use crate::last_seen::{FINAL_FLUSH_TIMEOUT, FlushTask};
use crate::signal_server::SignalStatus;
use crate::startup::{self, Prepared, Started};
use crate::{http, log_safe, logging, shutdown};

/// Hard bound from the signal to process exit (02 §7), split into
/// [`DRAIN_TIMEOUT`] + [`FINAL_FLUSH_TIMEOUT`] + [`DB_CLOSE_TIMEOUT`] +
/// [`RUNTIME_SHUTDOWN_GRACE`].
/// The signal server stops during the drain share, in parallel with HTTP
/// (its own write-behind flush is bounded by 5 s), and before the database
/// closes, so its last registrations still reach the store.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
/// Blocking tasks still running when the runtime is dropped get this long.
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// Closing the database pool.
const DB_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
/// Inside the runtime: HTTP drain, the last `last_seen_at` flush, database close.
const IN_RUNTIME_DEADLINE: Duration = SHUTDOWN_DEADLINE.saturating_sub(RUNTIME_SHUTDOWN_GRACE);
/// HTTP drain share of the deadline.
pub const DRAIN_TIMEOUT: Duration = IN_RUNTIME_DEADLINE
    .saturating_sub(DB_CLOSE_TIMEOUT)
    .saturating_sub(FINAL_FLUSH_TIMEOUT);

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
    let (signal_status, signal_task) = signal.map(|s| (s.status, s.task)).unzip();
    let apps = http::apps(
        Arc::clone(&state),
        admin_routes,
        &token,
        signal_status.clone(),
    );
    let serving = tokio::spawn({
        let token = token.clone();
        async move { listeners.serve(apps, &http_config, token).await }
    });
    let last_seen = FlushTask::start(&state, token.clone());
    tracing::info!("ready");
    // The last `last_seen_at` flush runs after HTTP and the signal server stopped.
    let close = move || async move {
        last_seen.finish().await;
        close_db(&state).await;
    };
    drive(serving, signal_task, signal_status.as_ref(), &token, close).await
}

/// Runs the serving phase to its end: waits for the shutdown request (or a fault), then
/// stops HTTP and the signal server in parallel and runs `close_db` only after both are
/// done, within the shutdown budget.
async fn drive<C: Future<Output = ()>>(
    mut serving: JoinHandle<Result<(), ServeError>>,
    mut signal_task: Option<JoinHandle<()>>,
    signal_status: Option<&SignalStatus>,
    token: &CancellationToken,
    close_db: impl FnOnce() -> C,
) -> anyhow::Result<ExitCode> {
    tokio::select! {
        joined = &mut serving => {
            // HTTP only stops on its own after the request, which can be pending next to
            // this branch (the select picks randomly): ask the token before cancelling it.
            let requested = token.is_cancelled();
            token.cancel();
            let stop = stop_signal(signal_task, signal_status);
            let _ = tokio::time::timeout(DRAIN_TIMEOUT, stop).await;
            close_db().await;
            return http_stopped(requested, joined);
        }
        joined = signal_ended(&mut signal_task) => {
            // The supervisor returns with the shutdown request (racing the token branch
            // here), or else it panicked. Its server is detached, not gone: wait for it
            // to drain before closing the database.
            if token.is_cancelled() {
                if let Err(err) = joined {
                    tracing::warn!(error = %err, "signal server supervisor failed");
                }
                signal_task = None;
            } else {
                token.cancel();
                let stop = async { tokio::join!(serving, stop_signal(None, signal_status)) };
                let _ = tokio::time::timeout(DRAIN_TIMEOUT, stop).await;
                close_db().await;
                return Err(match joined {
                    Ok(()) => anyhow!("signal server supervisor stopped unexpectedly"),
                    Err(err) => anyhow::Error::new(err).context("signal server supervisor failed"),
                });
            }
        }
        () = token.cancelled() => {}
    }

    let finish = async {
        let (joined, ()) = tokio::join!(serving, stop_signal(signal_task, signal_status));
        close_db().await;
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

/// Outcome of the HTTP server ending before the finish step observed the shutdown request.
/// Stopping cleanly after `requested` is the graceful shutdown; anything else is a fault.
fn http_stopped(
    requested: bool,
    joined: Result<Result<(), ServeError>, JoinError>,
) -> anyhow::Result<ExitCode> {
    match joined {
        Ok(Ok(())) if requested => {
            tracing::info!("shutdown complete");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(stopped_early(other)),
    }
}

/// The server only returns without error after shutdown was requested; anything else is a
/// fault.
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use tokio::task::AbortHandle;

    use super::*;

    #[test]
    fn shutdown_budget_adds_up_to_the_deadline() {
        assert_eq!(
            DRAIN_TIMEOUT + FINAL_FLUSH_TIMEOUT + DB_CLOSE_TIMEOUT + RUNTIME_SHUTDOWN_GRACE,
            SHUTDOWN_DEADLINE
        );
        assert!(!DRAIN_TIMEOUT.is_zero());
    }

    /// `drive` picks randomly among simultaneously ready branches, so a race between them
    /// is exercised over many rounds (a wrong outcome shows up with probability
    /// 1 - 2^-rounds).
    const RACE_ROUNDS: usize = 64;

    /// What `drive` observed about the database close.
    #[derive(Clone, Default)]
    struct Probe {
        closed: Arc<AtomicBool>,
        signal_drained: Arc<AtomicBool>,
        closed_after_signal: Arc<AtomicBool>,
    }

    impl Probe {
        fn closer(&self) -> impl FnOnce() -> std::future::Ready<()> {
            let probe = self.clone();
            move || {
                probe.closed.store(true, Ordering::SeqCst);
                let drained = probe.signal_drained.load(Ordering::SeqCst);
                probe.closed_after_signal.store(drained, Ordering::SeqCst);
                std::future::ready(())
            }
        }
    }

    /// An HTTP server that stops gracefully once `token` is cancelled.
    fn http_until_cancelled(token: &CancellationToken) -> JoinHandle<Result<(), ServeError>> {
        let token = token.clone();
        tokio::spawn(async move {
            token.cancelled().await;
            Ok(())
        })
    }

    /// A supervisor that returns once `token` is cancelled and the (slow) server drained.
    fn signal_until_cancelled(token: &CancellationToken, probe: &Probe) -> JoinHandle<()> {
        let (token, drained) = (token.clone(), Arc::clone(&probe.signal_drained));
        tokio::spawn(async move {
            token.cancelled().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
            drained.store(true, Ordering::SeqCst);
        })
    }

    /// Lets spawned tasks run until all of them finished, so `drive` finds every ready
    /// branch ready on its first poll.
    async fn settle(tasks: &[AbortHandle]) {
        while tasks.iter().any(|task| !task.is_finished()) {
            tokio::task::yield_now().await;
        }
    }

    fn exit_code_is_success(code: ExitCode) -> bool {
        format!("{code:?}") == format!("{:?}", ExitCode::SUCCESS)
    }

    #[tokio::test]
    async fn http_stop_after_the_shutdown_request_is_graceful() {
        for _ in 0..RACE_ROUNDS {
            // Arrange: shutdown requested and HTTP already stopped when `drive` polls.
            let token = CancellationToken::new();
            let probe = Probe::default();
            let serving = http_until_cancelled(&token);
            token.cancel();
            settle(&[serving.abort_handle()]).await;

            // Act
            let outcome = drive(serving, None, None, &token, probe.closer()).await;

            // Assert
            assert!(exit_code_is_success(outcome.expect("graceful stop")));
            assert!(probe.closed.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn supervisor_end_after_the_shutdown_request_is_graceful() {
        for _ in 0..RACE_ROUNDS {
            // Arrange: the supervisor returns as part of the requested shutdown.
            let token = CancellationToken::new();
            let probe = Probe::default();
            let serving = http_until_cancelled(&token);
            let signal = signal_until_cancelled(&token, &probe);
            token.cancel();
            settle(&[serving.abort_handle(), signal.abort_handle()]).await;

            // Act
            let outcome = drive(serving, Some(signal), None, &token, probe.closer()).await;

            // Assert
            assert!(exit_code_is_success(outcome.expect("graceful stop")));
            assert!(probe.closed_after_signal.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn database_closes_after_the_signal_server_drained_when_http_stops_first() {
        for _ in 0..RACE_ROUNDS {
            // Arrange: HTTP stops at once, the signal server needs a moment.
            let token = CancellationToken::new();
            let probe = Probe::default();
            let serving = http_until_cancelled(&token);
            let signal = signal_until_cancelled(&token, &probe);
            token.cancel();
            settle(&[serving.abort_handle()]).await;

            // Act
            let outcome = drive(serving, Some(signal), None, &token, probe.closer()).await;

            // Assert
            assert!(outcome.is_ok());
            assert!(probe.closed_after_signal.load(Ordering::SeqCst));
        }
    }

    #[tokio::test]
    async fn http_stop_without_a_shutdown_request_is_an_error() {
        // Arrange: HTTP returns although nobody asked it to.
        let token = CancellationToken::new();
        let probe = Probe::default();
        let signal = signal_until_cancelled(&token, &probe);
        let serving = tokio::spawn(async { Ok(()) });

        // Act
        let outcome = drive(serving, Some(signal), None, &token, probe.closer()).await;

        // Assert: a clear error, and the rest still shut down in order.
        let err = outcome.expect_err("unexpected stop must fail");
        assert_eq!(err.to_string(), "HTTP server stopped unexpectedly");
        assert!(token.is_cancelled());
        assert!(probe.closed_after_signal.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn http_failure_without_a_shutdown_request_is_reported() {
        // Arrange
        let token = CancellationToken::new();
        let probe = Probe::default();
        let serving = tokio::spawn(async { Err(ServeError::InvalidAdminPublicUrl) });

        // Act
        let outcome = drive(serving, None, None, &token, probe.closer()).await;

        // Assert
        let err = outcome.expect_err("serve error must fail");
        assert!(err.downcast_ref::<ServeError>().is_some(), "{err:#}");
        assert!(probe.closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn http_failure_during_shutdown_is_still_reported() {
        // Arrange: shutdown requested, but HTTP ends with an error.
        let token = CancellationToken::new();
        let probe = Probe::default();
        let serving = tokio::spawn({
            let token = token.clone();
            async move {
                token.cancelled().await;
                Err(ServeError::InvalidAdminPublicUrl)
            }
        });
        token.cancel();
        settle(&[serving.abort_handle()]).await;

        // Act
        let outcome = drive(serving, None, None, &token, probe.closer()).await;

        // Assert
        assert!(outcome.is_err());
        assert!(probe.closed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn supervisor_end_without_a_shutdown_request_is_an_error() {
        // Arrange: the supervisor ends on its own (a panic would look the same).
        let token = CancellationToken::new();
        let probe = Probe::default();
        let serving = http_until_cancelled(&token);
        let signal = tokio::spawn(async {});

        // Act
        let outcome = drive(serving, Some(signal), None, &token, probe.closer()).await;

        // Assert
        let err = outcome.expect_err("unexpected supervisor end must fail");
        assert_eq!(
            err.to_string(),
            "signal server supervisor stopped unexpectedly"
        );
        assert!(probe.closed.load(Ordering::SeqCst));
    }
}
