//! The login event of a client password login (04 §7 `login_events`), written from
//! [`record`] only.
//!
//! Recorded: every attempt that was verified — `success`, `invalid_credentials` (wrong
//! password, unknown or non-local account alike), `disabled`, `unverified` — and `locked`
//! for the verified failure that started a lockout. Not recorded: refused attempts
//! (429: IP limit or lockout; 503: hasher busy), because nothing was verified and their
//! number is unbounded, so recording them would hand attackers one database write per
//! request; malformed requests (400); server failures (500, logged instead).
//!
//! `user_id` is set on success only: a failure row must not need an account lookup (which
//! would make the failure paths differ), and `username_attempted` names it anyway.

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginClient, LoginMethod, LoginResult, NewLoginEvent};
use deskhaven_core::state::AppState;

use super::request::AccountLogin;

/// Writes the event. Best effort: a failed write is logged and never changes the reply.
/// No password, token or request body is stored or logged.
pub async fn record(
    state: &AppState,
    client: ClientAddr,
    request: &AccountLogin,
    user_id: Option<i64>,
    result: LoginResult,
) {
    let event = NewLoginEvent {
        user_id,
        username_attempted: &request.username,
        method: LoginMethod::Password,
        result,
        client: LoginClient::Rustdesk,
        rustdesk_id: Some(&request.device.rustdesk_id),
        ip: Some(client.addr),
    };
    let (result, ip) = (result.as_str(), client.addr);
    if let Err(err) = state.login_events.record(&event).await {
        tracing::error!(result, %ip, error = %err, "cannot write the login event");
    }
    // The attempted username stays out of the log: people type passwords into it.
    if result == LoginResult::Success.as_str() {
        tracing::info!(user_id, result, %ip, "client login");
    } else {
        tracing::debug!(result, %ip, "client login failed");
    }
}
