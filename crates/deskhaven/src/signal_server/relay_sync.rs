//! Settings -> running signal server: `signal.relay_servers` and `signal.always_use_relay`
//! changes are applied through the server's `SignalHandle`, debounced, one at a time (the
//! handle requires serialized relay updates), on a task of their own so neither the settings
//! writer nor the server loop ever waits for relay DNS.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::Snapshot;
use deskhaven_signal::{SignalError, SignalHandle};
use tokio::sync::watch;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::RelaySettings;

/// Changes arriving within this window are applied together, as the latest snapshot.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(200);
/// A relay list the server rejected (entries that do not resolve) is retried this often
/// while the settings still ask for it.
pub(super) const RETRY_REJECTED: Duration = Duration::from_secs(30);

/// Result of one attempt to bring the server to the wanted settings.
enum Outcome {
    /// What the server now runs with; `retry` when the relay list was rejected.
    Applied { now: RelaySettings, retry: bool },
    /// The server is gone.
    Stopped,
}

/// Follows `settings` until `stop` fires, the server stops or the settings service is gone.
/// `applied` is what the server was started with; only changes seen after `settings`'
/// current version are applied.
pub(super) async fn run(
    handle: SignalHandle,
    mut settings: watch::Receiver<Arc<Snapshot>>,
    applied: RelaySettings,
    stop: CancellationToken,
) {
    let mut applied = applied;
    let mut retry = false;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            changed = settings.changed() => if changed.is_err() { return },
            () = sleep(RETRY_REJECTED), if retry => {}
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep(DEBOUNCE) => {}
        }
        let wanted = RelaySettings::from_snapshot(&settings.borrow_and_update());
        match apply(&handle, &applied, wanted).await {
            Outcome::Applied { now, retry: again } => {
                applied = now;
                retry = again;
            }
            Outcome::Stopped => return,
        }
    }
}

async fn apply(handle: &SignalHandle, applied: &RelaySettings, wanted: RelaySettings) -> Outcome {
    if wanted.always_use_relay != applied.always_use_relay {
        if handle
            .set_always_use_relay(wanted.always_use_relay)
            .is_err()
        {
            return Outcome::Stopped;
        }
        tracing::info!(
            always_use_relay = wanted.always_use_relay,
            "signal always_use_relay applied"
        );
    }
    if wanted.servers == applied.servers {
        return Outcome::Applied {
            now: wanted,
            retry: false,
        };
    }
    match handle.set_relay_servers(wanted.servers.clone()).await {
        Ok(accepted) => {
            tracing::info!(relay_servers = ?accepted, "signal relay servers applied");
            Outcome::Applied {
                now: wanted,
                retry: false,
            }
        }
        Err(SignalError::RelayServersRejected { rejected }) => {
            tracing::warn!(
                ?rejected,
                retry_in = ?RETRY_REJECTED,
                "signal relay servers not applied (entries do not resolve); keeping the previous list"
            );
            let now = RelaySettings {
                servers: applied.servers.clone(),
                ..wanted
            };
            Outcome::Applied { now, retry: true }
        }
        Err(_) => Outcome::Stopped,
    }
}
