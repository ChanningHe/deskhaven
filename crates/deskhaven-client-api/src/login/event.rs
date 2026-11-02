//! Login events (04 §7 `login_events`): one record per account login attempt, written from
//! [`record`] only. Carries no password, token or request body.

use std::net::IpAddr;

use deskhaven_core::state::AppState;

/// `login_events.method`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Password,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
        }
    }
}

/// `login_events.result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginResult {
    Success,
    /// Wrong password, unknown or non-local account (one result: no enumeration in the log
    /// either).
    InvalidCredentials,
    Disabled,
    Unverified,
    /// Per-IP limit or account lockout; nothing was verified.
    Throttled,
    /// Password hasher overloaded; nothing was verified.
    Busy,
    /// Server-side failure.
    Error,
}

impl LoginResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Disabled => "disabled",
            Self::Unverified => "unverified",
            Self::Throttled => "throttled",
            Self::Busy => "busy",
            Self::Error => "error",
        }
    }
}

/// One `login_events` row (client `rustdesk`).
#[derive(Debug)]
pub struct LoginEvent<'a> {
    /// Known on success only (a failure must not reveal whether the account exists).
    pub user_id: Option<i64>,
    #[expect(
        dead_code,
        reason = "stored in login_events once the table exists (M2-02)"
    )]
    pub username_attempted: &'a str,
    pub method: Method,
    pub result: LoginResult,
    pub rustdesk_id: &'a str,
    pub ip: IpAddr,
}

/// Records `event`. Best effort: a failed write is logged and never fails the login.
pub async fn record(state: &AppState, event: &LoginEvent<'_>) {
    // TODO(M2-02): login_events — insert `event` through the login-event repository of
    // `state` here (the table does not exist yet). Until then the log is the only record.
    let _ = state;
    // The attempted username stays out of the log: users type passwords into it.
    let (user_id, result, ip) = (event.user_id, event.result.as_str(), event.ip);
    let (method, rustdesk_id) = (event.method.as_str(), event.rustdesk_id);
    if event.result == LoginResult::Success {
        tracing::info!(user_id, method, result, %ip, rustdesk_id, "client login");
    } else {
        tracing::debug!(method, result, %ip, rustdesk_id, "client login failed");
    }
}
