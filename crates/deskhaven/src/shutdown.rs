use anyhow::Context;
use tokio_util::sync::CancellationToken;

#[cfg(unix)]
pub fn spawn_signal_listener(token: CancellationToken) -> anyhow::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut term = signal(SignalKind::terminate()).context("cannot listen for SIGTERM")?;
    let mut int = signal(SignalKind::interrupt()).context("cannot listen for SIGINT")?;
    tokio::spawn(async move {
        let name = tokio::select! {
            _ = term.recv() => "SIGTERM",
            _ = int.recv() => "SIGINT",
            () = token.cancelled() => return,
        };
        tracing::info!(signal = name, "shutdown requested");
        token.cancel();
    });
    Ok(())
}

#[cfg(not(unix))]
pub fn spawn_signal_listener(token: CancellationToken) -> anyhow::Result<()> {
    tokio::spawn(async move {
        tokio::select! {
            result = tokio::signal::ctrl_c() => match result {
                Ok(()) => tracing::info!(signal = "ctrl-c", "shutdown requested"),
                Err(err) => tracing::error!(error = %err, "cannot listen for ctrl-c; shutting down"),
            },
            () = token.cancelled() => return,
        }
        token.cancel();
    });
    Ok(())
}
