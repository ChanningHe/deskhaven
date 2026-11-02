//! The login event of a console sign-in step (04 §7 `login_events`, `client = console`).
//!
//! Recorded, as for the client: every verified attempt (`success`, `invalid_credentials`
//! for wrong password and unknown account alike, `disabled`, `unverified`, `locked` for the
//! failure that started a lockout, `tfa_required` when a challenge was issued,
//! `invalid_code` / `invalid_challenge` for the second step with method `totp` or
//! `recovery`), and OIDC outcomes (method `oidc`, the attempted name `provider:subject`).
//! Not recorded: throttled or busy refusals (unbounded in number, nothing verified),
//! malformed requests and server failures (logged).

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginClient, LoginMethod, LoginResult, NewLoginEvent};
use deskhaven_core::state::AppState;

/// Writes the event. Best effort: a failed write is logged and never changes the reply.
/// No password, code, token or request body is stored or logged.
pub(crate) async fn record(
    state: &AppState,
    client: ClientAddr,
    attempted: &str,
    method: LoginMethod,
    user_id: Option<i64>,
    result: LoginResult,
) {
    let event = NewLoginEvent {
        user_id,
        username_attempted: attempted,
        method,
        result,
        client: LoginClient::Console,
        rustdesk_id: None,
        ip: Some(client.addr),
    };
    let (result, ip) = (result.as_str(), client.addr);
    if let Err(err) = state.login_events.record(&event).await {
        tracing::error!(result, %ip, error = %err, "cannot write the login event");
    }
    // The attempted username stays out of the log: people type passwords into it.
    if result == LoginResult::Success.as_str() {
        tracing::info!(user_id, result, %ip, "console login");
    } else {
        tracing::debug!(result, %ip, "console login step failed or pending");
    }
}
