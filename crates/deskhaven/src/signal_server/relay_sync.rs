//! Settings changes reach the running server debounced and one at a time (relay updates must be
//! serialized), on a task of their own, so neither the settings writer nor the server loop
//! waits for relay DNS. A rejected list keeps the server's previous one and is retried.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::Snapshot;
use deskhaven_signal::{SignalError, SignalHandle};
use tokio::sync::watch;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;

use super::SignalSettings;

/// Changes arriving within this window are applied together, as the latest snapshot.
pub(super) const DEBOUNCE: Duration = Duration::from_millis(200);
pub(super) const RETRY_REJECTED: Duration = Duration::from_secs(30);

struct State {
    applied: SignalSettings,
    rejected: Option<Vec<String>>,
}

/// Only changes seen after `settings`' current version are applied; `stop` also interrupts an
/// update waiting for DNS.
pub(super) async fn run(
    handle: SignalHandle,
    mut settings: watch::Receiver<Arc<Snapshot>>,
    applied: SignalSettings,
    stop: CancellationToken,
) {
    let mut state = State {
        applied,
        rejected: None,
    };
    loop {
        let retry = state.rejected.is_some();
        tokio::select! {
            () = stop.cancelled() => return,
            changed = settings.changed() => if changed.is_err() { return },
            () = sleep(RETRY_REJECTED), if retry => {}
        }
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep(DEBOUNCE) => {}
        }
        let wanted = SignalSettings::from_snapshot(&settings.borrow_and_update());
        state = tokio::select! {
            () = stop.cancelled() => return,
            next = apply(&handle, state, wanted) => match next {
                Some(next) => next,
                None => return,
            },
        };
    }
}

/// The switches go first, so `require_kx_v1` never waits for the relay list's DNS.
async fn apply(handle: &SignalHandle, state: State, wanted: SignalSettings) -> Option<State> {
    let State { applied, rejected } = state;
    if wanted.always_use_relay != applied.always_use_relay {
        handle.set_always_use_relay(wanted.always_use_relay).ok()?;
        tracing::info!(
            always_use_relay = wanted.always_use_relay,
            "signal always_use_relay applied"
        );
    }
    if wanted.require_kx_v1 != applied.require_kx_v1 {
        handle.set_require_kx_v1(wanted.require_kx_v1).ok()?;
        tracing::info!(
            require_kx_v1 = wanted.require_kx_v1,
            "signal require_kx_v1 applied"
        );
    }
    if wanted.servers == applied.servers {
        return Some(State {
            applied: wanted,
            rejected: None,
        });
    }
    match handle.set_relay_servers(wanted.servers.clone()).await {
        Ok(accepted) => {
            tracing::info!(relay_servers = ?accepted, "signal relay servers applied");
            Some(State {
                applied: wanted,
                rejected: None,
            })
        }
        Err(SignalError::RelayServersRejected { rejected: entries }) => {
            log_rejection(rejected.as_deref(), &entries);
            let applied = SignalSettings {
                servers: applied.servers,
                ..wanted
            };
            Some(State {
                applied,
                rejected: Some(entries),
            })
        }
        Err(_) => None,
    }
}

fn log_rejection(previous: Option<&[String]>, rejected: &[String]) {
    if is_new_rejection(previous, rejected) {
        tracing::warn!(
            ?rejected,
            retry_in = ?RETRY_REJECTED,
            "signal relay servers not applied (entries do not resolve); keeping the previous list"
        );
    } else {
        tracing::debug!(?rejected, "signal relay servers still do not resolve");
    }
}

pub(super) fn is_new_rejection(previous: Option<&[String]>, rejected: &[String]) -> bool {
    previous != Some(rejected)
}
