//! `<public_url>/api/oidc/callback`: the browser comes back from the IdP with
//! `?code=&state=` (or `?error=&state=`).
//!
//! - `GET` completes nothing. It shows which device the flow would sign in (RustDesk id,
//!   host name and OS the device gave at `/api/oidc/auth`, escaped) and a button. This stops
//!   cross-device consent phishing: an attacker who sends their flow's authorization URL to
//!   someone already signed in at the IdP gets a page naming the attacker's device, not a
//!   silent login (`deskhaven_core::oidc` `preview`).
//! - `POST` (the button; the form posts back to the same URL, query included, so the code
//!   and state are never copied into the page) completes the flow once (single use).
//!
//! - A **console** flow (M3-02) is not completed here: `GET` answers `303` to the
//!   console's `/login/oidc` with the IdP's query, where the browser that started the login
//!   (holding the flow's binding cookie on the console origin) completes it through the
//!   admin API. `POST` treats a console flow as an invalid link and leaves it untouched.
//!
//! Otherwise always 200 with a small HTML page (`page`): the reader is a person, and the
//! HTTP stack replaces the body of any non-JSON 4xx/5xx under `/api`, which would hide the
//! message. The outcome is in the page, the log and the login event.
//!
//! | case | `GET` | `POST` |
//! |---|---|---|
//! | live client flow | confirmation | the outcome: success, or the failure's message |
//! | live console flow | 303 to the console | invalid link |
//! | IdP `error` or no `code` | "stop waiting?" confirmation | denied (the client stops polling) |
//! | unknown, expired or already used state | invalid link | invalid link |
//! | server error | generic | generic |
//!
//! Login events: a failed flow is recorded at the `POST` (method `oidc`, the browser's
//! address, the flow's device); a successful one when the client collects its session
//! (`query`). Unknown states are not recorded (nothing was verified; their number is
//! unbounded).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, CallbackPreview, CallbackReport, DeviceSummary, FlowKind,
};
use deskhaven_core::state::AppState;

use super::failure_message;
use super::page::{self, escape};
use super::params::param;
use crate::client::Client;
use crate::login::event::{self, Attempted};

const SUCCESS: &str = "Login successful. You can close this window and return to RustDesk.";
const INVALID_LINK: &str =
    "This login link is invalid, expired or already used. Start the login again from RustDesk.";
const SERVER_ERROR: &str = "The login could not be completed. Please try again later.";
const WARNING: &str = "Only continue if you started this login on that device yourself.";
/// The console page that completes a console login (`web/src/routes/login.oidc.tsx`).
pub const CONSOLE_COMPLETION_PATH: &str = "/login/oidc";

/// The callback query, decoded.
struct Query {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

impl Query {
    fn parse(raw: Option<&str>) -> Self {
        Self {
            state: param(raw, "state"),
            code: param(raw, "code"),
            error: param(raw, "error"),
        }
    }

    fn params(&self) -> CallbackParams<'_> {
        CallbackParams {
            state: self.state.as_deref(),
            code: self.code.as_deref(),
            error: self.error.as_deref(),
        }
    }

    /// The IdP did not sign the user in.
    fn is_denied(&self) -> bool {
        self.error.is_some() || self.code.is_none()
    }
}

/// `GET`: the confirmation page of a client flow, or the way on into the console for a
/// console flow; nothing changes.
pub async fn confirm(State(state): State<Arc<AppState>>, RawQuery(raw): RawQuery) -> Response {
    let query = Query::parse(raw.as_deref());
    match state.oidc.preview_callback(query.state.as_deref()).await {
        Ok(CallbackPreview {
            provider,
            device: Some(device),
        }) => confirmation(&provider, &device, query.is_denied()),
        Ok(CallbackPreview { device: None, .. }) => {
            to_console(&state.console_url, raw.as_deref().unwrap_or_default())
        }
        Err(CallbackError::InvalidState) => page::message(INVALID_LINK),
        Err(err) => {
            tracing::error!(error = %err, "OIDC callback preview failed");
            page::message(SERVER_ERROR)
        }
    }
}

/// `POST`: completes the flow.
pub async fn complete(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    RawQuery(raw): RawQuery,
) -> Response {
    let query = Query::parse(raw.as_deref());
    let message = match state.oidc.complete_callback(&query.params()).await {
        Ok(report) => {
            record(&state, client, &report).await;
            match report.outcome {
                Ok(_) => SUCCESS,
                Err(failure) => failure_message(failure),
            }
        }
        Err(CallbackError::InvalidState) => INVALID_LINK,
        Err(err) => {
            tracing::error!(error = %err, "OIDC callback failed");
            SERVER_ERROR
        }
    };
    page::message(message)
}

/// A console login completes in the console itself (the browser that started it holds
/// the flow's binding cookie there): `303` to `<console>/login/oidc?<the IdP's query>`.
/// Only the query is copied; the target is fixed, so this is no open redirect.
fn to_console(console_url: &str, raw_query: &str) -> Response {
    let location = format!("{console_url}{CONSOLE_COMPLETION_PATH}?{raw_query}");
    match HeaderValue::from_str(&location) {
        Ok(location) => (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, location),
                (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            ],
        )
            .into_response(),
        Err(_) => page::message(INVALID_LINK),
    }
}

fn confirmation(provider: &str, device: &DeviceSummary, denied: bool) -> Response {
    let provider = escape(provider);
    let details: Vec<String> = [device.name.as_deref(), device.os.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(escape)
        .collect();
    let target = match details.is_empty() {
        true => format!("RustDesk device {}", escape(&device.rustdesk_id)),
        false => format!(
            "RustDesk device {} ({})",
            escape(&device.rustdesk_id),
            details.join(", ")
        ),
    };
    if denied {
        let question = format!("{provider} did not sign you in. Tell {target} to stop waiting?");
        return page::confirm(&question, "", "Stop waiting");
    }
    page::confirm(
        &format!("Sign in {target} with {provider}?"),
        WARNING,
        "Sign in",
    )
}

/// The failure of a client flow; successes are recorded when the session is collected.
async fn record(state: &AppState, client: ClientAddr, report: &CallbackReport) {
    let (FlowKind::Client { rustdesk_id }, Err(failure)) = (&report.flow, &report.outcome) else {
        return;
    };
    let username = match &report.subject {
        Some(subject) => format!("{}:{subject}", report.provider),
        None => report.provider.clone(),
    };
    let attempted = Attempted {
        username: &username,
        rustdesk_id,
        method: LoginMethod::Oidc,
    };
    event::record(state, client, &attempted, None, LoginResult::from(*failure)).await;
}
