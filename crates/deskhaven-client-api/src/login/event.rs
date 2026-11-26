//! Refused attempts (429, 503) are not recorded: nothing was verified and their number is
//! unbounded, so recording them would hand attackers one database write per request. A
//! failure row needs no account lookup, which would make the failure paths differ.

use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginClient, LoginMethod, LoginResult, NewLoginEvent};
use deskhaven_core::state::AppState;

pub struct Attempted<'a> {
    pub username: &'a str,
    pub rustdesk_id: &'a str,
    pub method: LoginMethod,
}

/// Best effort: a failed write is logged and never changes the reply.
pub async fn record(
    state: &AppState,
    client: ClientAddr,
    attempted: &Attempted<'_>,
    user_id: Option<i64>,
    result: LoginResult,
) {
    let event = NewLoginEvent {
        user_id,
        username_attempted: attempted.username,
        method: attempted.method,
        result,
        client: LoginClient::Rustdesk,
        rustdesk_id: Some(attempted.rustdesk_id),
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
        tracing::debug!(result, %ip, "client login step failed or pending");
    }
}
