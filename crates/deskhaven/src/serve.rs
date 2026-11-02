//! `deskhaven serve`: configuration, logging, startup, serving until
//! SIGTERM/SIGINT, then a bounded shutdown (02 §7).
//!
//! Exit codes: 0 after a clean shutdown (also when a signal arrives during
//! startup), 1 on any startup failure or when shutdown overruns its deadline.

use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, anyhow};
use deskhaven_config::bootstrap::{Bootstrap, LoadOptions, load};
use deskhaven_http::ServeError;
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;

use crate::startup::{self, Prepared, Started};
use crate::state::AppState;
use crate::{http, log_safe, logging, shutdown};

/// Whole shutdown budget, counted from the signal.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);
/// Kept back from the HTTP drain for closing the database pool.
const DB_CLOSE_RESERVE: Duration = Duration::from_secs(2);
/// HTTP drain share of [`SHUTDOWN_DEADLINE`].
pub const DRAIN_TIMEOUT: Duration = SHUTDOWN_DEADLINE.saturating_sub(DB_CLOSE_RESERVE);
/// Blocking tasks still running when the runtime is dropped get this long.
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

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
        () = token.cancelled() => {
            tracing::info!("shutdown requested before startup completed");
            return Ok(ExitCode::SUCCESS);
        }
        started = startup::start(config, prepared, DRAIN_TIMEOUT) => started?,
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
    } = started;
    let apps = http::apps(&state, &token);
    let mut serving = tokio::spawn({
        let token = token.clone();
        async move { listeners.serve(apps, &http_config, token).await }
    });
    tracing::info!("ready");

    tokio::select! {
        joined = &mut serving => {
            close_db(&state).await;
            return Err(stopped_early(joined));
        }
        () = token.cancelled() => {}
    }

    let finish = async {
        let joined = serving.await;
        close_db(&state).await;
        joined
    };
    match tokio::time::timeout(SHUTDOWN_DEADLINE, finish).await {
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

/// The server only returns after shutdown was requested; anything else is a fault.
fn stopped_early(joined: Result<Result<(), ServeError>, JoinError>) -> anyhow::Error {
    match joined {
        Ok(Ok(())) => anyhow!("HTTP server stopped unexpectedly"),
        Ok(Err(err)) => err.into(),
        Err(err) => anyhow::Error::new(err).context("HTTP server task failed"),
    }
}

async fn close_db(state: &AppState) {
    match state.db.close_by_ref().await {
        Ok(()) => tracing::info!("database closed"),
        Err(err) => {
            tracing::warn!(error = %log_safe::render_error(&err), "closing the database failed")
        }
    }
}
