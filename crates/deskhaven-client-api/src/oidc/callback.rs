//! `GET` completes nothing: it names the device the flow would sign in and shows a button,
//! which stops cross-device consent phishing. The form posts back to the same URL, so the
//! code and state are never copied into the page. Always 200 with an HTML page: the HTTP
//! stack replaces the body of any non-JSON 4xx/5xx under `/api`.

use std::sync::Arc;

use axum::extract::{RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::IdHolder;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, CallbackPreview, CallbackReport, DeviceSummary, FlowKind,
    PreviewTarget,
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
const WARNING: &str = "Only continue if you started this login on that device yourself and \
                       its RustDesk ID is the one your RustDesk client shows.";
/// Nothing ties an unregistered id to the device that started the login.
const UNREGISTERED_WARNING: &str = "This RustDesk ID has not connected to this server yet, so \
                                    the server cannot tell which device started this login. \
                                    If your RustDesk client does not show this ID, do not \
                                    continue.";
const ID_TAKEN: &str = "This login cannot be completed: its RustDesk ID belongs to another \
                        device on this server. Nothing was signed in.";
pub const CONSOLE_COMPLETION_PATH: &str = "/login/oidc";
pub const CONSOLE_LINK_PATH: &str = "/me/oidc-link";

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

    fn is_denied(&self) -> bool {
        self.error.is_some() || self.code.is_none()
    }
}

pub async fn confirm(State(state): State<Arc<AppState>>, RawQuery(raw): RawQuery) -> Response {
    let query = Query::parse(raw.as_deref());
    let raw_query = raw.as_deref().unwrap_or_default();
    match state.oidc.preview_callback(query.state.as_deref()).await {
        Ok(CallbackPreview {
            provider,
            target: PreviewTarget::Device(device),
        }) => {
            let ask = match query.is_denied() {
                true => Ask::StopWaiting,
                false => match admit(&state, &device).await {
                    Ok(registered) => Ask::SignIn { registered },
                    Err(message) => return page::message(message),
                },
            };
            confirmation(&provider, &device, ask)
        }
        Ok(CallbackPreview {
            target: PreviewTarget::Console,
            ..
        }) => to_console(&state.console_url, CONSOLE_COMPLETION_PATH, raw_query),
        Ok(CallbackPreview {
            target: PreviewTarget::Link,
            ..
        }) => to_console(&state.console_url, CONSOLE_LINK_PATH, raw_query),
        Err(CallbackError::InvalidState) => page::message(INVALID_LINK),
        Err(err) => {
            tracing::error!(error = %err, "OIDC callback preview failed");
            page::message(SERVER_ERROR)
        }
    }
}

pub async fn complete(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    RawQuery(raw): RawQuery,
) -> Response {
    let query = Query::parse(raw.as_deref());
    if !query.is_denied()
        && let Err(message) = recheck(&state, query.state.as_deref()).await
    {
        return page::message(message);
    }
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

async fn admit(state: &AppState, device: &DeviceSummary) -> Result<bool, &'static str> {
    match state
        .devices
        .id_holder(&device.rustdesk_id, &device.uuid)
        .await
    {
        Ok(IdHolder::ThisDevice) => Ok(true),
        Ok(IdHolder::Nobody) => Ok(false),
        Ok(IdHolder::OtherDevice) => {
            let id = device.rustdesk_id.as_str();
            tracing::warn!(
                rustdesk_id = id,
                "OIDC confirmation refused: the id belongs to another device"
            );
            Err(ID_TAKEN)
        }
        // The signal server did not answer in time (logged there).
        Ok(IdHolder::Unknown) => Err(SERVER_ERROR),
        Err(err) => {
            tracing::error!(error = %err, "OIDC device check failed");
            Err(SERVER_ERROR)
        }
    }
}

/// Anything but a live client flow passes: `complete_callback` gives its answer.
async fn recheck(state: &AppState, flow_state: Option<&str>) -> Result<(), &'static str> {
    match state.oidc.preview_callback(flow_state).await {
        Ok(CallbackPreview {
            target: PreviewTarget::Device(device),
            ..
        }) => admit(state, &device).await.map(drop),
        _ => Ok(()),
    }
}

/// Only the query is copied and the target is fixed, so this is no open redirect.
fn to_console(console_url: &str, path: &str, raw_query: &str) -> Response {
    let location = format!("{console_url}{path}?{raw_query}");
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    StopWaiting,
    SignIn { registered: bool },
}

struct Confirmation {
    question: String,
    notes: Vec<String>,
    button: &'static str,
}

fn confirmation(provider: &str, device: &DeviceSummary, ask: Ask) -> Response {
    let parts = confirmation_parts(provider, device, ask);
    page::confirm(&parts.question, &parts.notes, parts.button)
}

/// The RustDesk ID comes first and in bold: the one thing the user can check against their
/// own client. Name and OS are whatever the device sent, so they are only a note, labelled
/// as unverified.
fn confirmation_parts(provider: &str, device: &DeviceSummary, ask: Ask) -> Confirmation {
    let provider = escape(provider);
    let target = format!(
        "RustDesk device <strong>{}</strong>",
        escape(&device.rustdesk_id)
    );
    let reported = reported(device);
    let warning = match ask {
        Ask::StopWaiting => {
            return Confirmation {
                question: format!("{provider} did not sign you in. Tell {target} to stop waiting?"),
                notes: reported.into_iter().collect(),
                button: "Stop waiting",
            };
        }
        Ask::SignIn { registered: true } => WARNING,
        Ask::SignIn { registered: false } => UNREGISTERED_WARNING,
    };
    Confirmation {
        question: format!("Sign in {target} with {provider}?"),
        notes: reported.into_iter().chain([warning.to_owned()]).collect(),
        button: "Sign in",
    }
}

fn reported(device: &DeviceSummary) -> Option<String> {
    let fields: Vec<String> = [("name", &device.name), ("system", &device.os)]
        .into_iter()
        .filter_map(|(label, value)| {
            let value = value.as_deref().filter(|v| !v.is_empty())?;
            Some(format!("{label} \u{201c}{}\u{201d}", escape(value)))
        })
        .collect();
    (!fields.is_empty()).then(|| {
        format!(
            "Reported by the device itself, not verified: {}.",
            fields.join(", ")
        )
    })
}

/// Successes are recorded when the session is collected.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: Option<&str>, os: Option<&str>) -> DeviceSummary {
        DeviceSummary {
            rustdesk_id: "123456789".to_owned(),
            uuid: "aGVsbG8=".to_owned(),
            name: name.map(str::to_owned),
            os: os.map(str::to_owned),
        }
    }

    #[test]
    fn the_question_names_the_rustdesk_id_and_the_self_description_is_only_a_note() {
        let shown = device(Some("Your work PC"), Some("linux"));

        let parts = confirmation_parts("corp", &shown, Ask::SignIn { registered: true });

        assert!(
            parts.question.contains("<strong>123456789</strong>"),
            "{}",
            parts.question
        );
        assert!(!parts.question.contains("Your work PC"));
        let reported = &parts.notes[0];
        assert!(reported.contains("Your work PC") && reported.contains("linux"));
        assert_eq!(parts.notes.last(), Some(&WARNING.to_owned()));
    }

    #[test]
    fn an_unregistered_id_gets_the_plain_warning_instead() {
        let shown = device(Some("host"), None);

        let parts = confirmation_parts("corp", &shown, Ask::SignIn { registered: false });

        assert_eq!(parts.notes.last(), Some(&UNREGISTERED_WARNING.to_owned()));
        assert!(!parts.notes.contains(&WARNING.to_owned()));
    }

    #[test]
    fn a_device_that_said_nothing_gets_no_reported_line_and_markup_is_escaped() {
        let registered = Ask::SignIn { registered: true };
        let silent = confirmation_parts("corp", &device(None, Some("")), registered);
        let hostile = confirmation_parts(
            "<i>p</i>",
            &device(Some("<b>x</b>"), None),
            Ask::StopWaiting,
        );

        assert_eq!(silent.notes, [WARNING.to_owned()]);
        assert!(hostile.question.contains("&lt;i&gt;p&lt;/i&gt;"));
        assert!(!hostile.question.contains("<i>"));
        assert!(hostile.notes[0].contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(!hostile.notes[0].contains("<b>"));
    }
}
