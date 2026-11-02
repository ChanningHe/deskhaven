//! `GET <public_url>/api/oidc/callback`: the browser comes back from the IdP with
//! `?code=&state=` (or `?error=&state=`). Answers a small static HTML page; nothing from
//! the request is echoed into it, and it loads nothing (`default-src 'none'`), is never
//! framed, cached or sent as a referrer (the URL carries the code and state).
//!
//! Always 200: the reader is a person, and the HTTP stack replaces the body of any non-JSON
//! 4xx/5xx under `/api` (`deskhaven_http` stack errors), which would hide the message. The
//! outcome is in the page, the log and the login event.
//!
//! | case | page |
//! |---|---|
//! | login completed (the client's next poll gets the session) | success |
//! | flow failed (denied, provider error, bad token, no account, disabled, …) | the failure's message |
//! | unknown, expired or already used state | invalid link |
//! | server error | generic |
//!
//! Login events: a failed flow is recorded here (method `oidc`, the browser's address, the
//! flow's device); a successful one when the client collects its session (`query`). An
//! unknown state is not recorded (nothing was verified, and their number is unbounded).

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, header};
use axum::response::{Html, IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::oidc::{CallbackError, CallbackParams, CallbackReport, FlowKind};
use deskhaven_core::state::AppState;
use deskhaven_http::error_chain;

use super::failure_message;
use super::params::param;
use crate::client::Client;
use crate::login::event::{self, Attempted};

const CSP: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
const SUCCESS: &str = "Login successful. You can close this window and return to RustDesk.";
const INVALID_LINK: &str =
    "This login link is invalid, expired or already used. Start the login again from RustDesk.";
const SERVER_ERROR: &str = "The login could not be completed. Please try again later.";

pub async fn callback(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    RawQuery(query): RawQuery,
) -> Response {
    let query = query.as_deref();
    let (state_param, code, error) = (
        param(query, "state"),
        param(query, "code"),
        param(query, "error"),
    );
    let request = CallbackParams {
        state: state_param.as_deref(),
        code: code.as_deref(),
        error: error.as_deref(),
    };
    let message = match state.oidc.complete_callback(&request).await {
        Ok(report) => {
            record(&state, client, &report).await;
            match report.outcome {
                Ok(_) => SUCCESS,
                Err(failure) => failure_message(failure),
            }
        }
        Err(CallbackError::InvalidState) => INVALID_LINK,
        Err(err) => {
            tracing::error!(error = %error_chain(&err), "OIDC callback failed");
            SERVER_ERROR
        }
    };
    page(message)
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

/// The page around a static message (never request data: no escaping needed).
fn page(message: &'static str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width\">\
         <title>deskhaven</title></head><body><p>{message}</p></body></html>"
    );
    let mut response = Html(html).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
