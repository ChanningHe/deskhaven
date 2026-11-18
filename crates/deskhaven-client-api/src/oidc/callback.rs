//! `<public_url>/api/oidc/callback`: the browser comes back from the IdP with
//! `?code=&state=` (or `?error=&state=`).
//!
//! - `GET` completes nothing. It shows which device the flow would sign in (RustDesk id
//!   first and in bold; host name and OS the device gave at `/api/oidc/auth`, escaped and
//!   labelled as unverified, since a device can call itself anything) and a button. This stops
//!   cross-device consent phishing: an attacker who sends their flow's authorization URL to
//!   someone already signed in at the IdP gets a page naming the attacker's device, not a
//!   silent login (`deskhaven_core::oidc` `preview`).
//! - `POST` (the button; the form posts back to the same URL, query included, so the code
//!   and state are never copied into the page) completes the flow once (single use).
//! - **Whose id** (see `oidc`): both check again who holds the flow's RustDesk id, since a
//!   device may have registered it in the minutes since `/api/oidc/auth`. An id registered
//!   with the flow's uuid gets the usual warning; one not registered yet a plain one that
//!   the server cannot vouch for it; one registered with another uuid no button, and its
//!   `POST` completes nothing (the flow is left to expire; the client gives up polling).
//!
//! - A **console** flow (M3-02) is not completed here: `GET` answers `303` to the
//!   console's `/login/oidc` with the IdP's query, where the browser that started the login
//!   (holding the flow's binding cookie on the console origin) completes it through the
//!   admin API. `POST` treats a console flow as an invalid link and leaves it untouched.
//! - A **link** flow (M4-09, a signed-in user linking an identity to their account) goes
//!   the same way to the console's `/me/oidc-link`, which names the provider and the
//!   account, and completes it for that account in that browser only.
//!
//! Otherwise always 200 with a small HTML page (`page`): the reader is a person, and the
//! HTTP stack replaces the body of any non-JSON 4xx/5xx under `/api`, which would hide the
//! message. The outcome is in the page, the log and the login event.
//!
//! | case | `GET` | `POST` |
//! |---|---|---|
//! | live client flow | confirmation | the outcome: success, or the failure's message |
//! | live client flow, id registered with another uuid | id taken | id taken, nothing completed |
//! | live console flow | 303 to the console's login completion | invalid link |
//! | live link flow | 303 to the console's link confirmation | invalid link |
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
/// Instead of [`WARNING`] for an id not registered here yet: nothing ties it to the device
/// that started the login.
const UNREGISTERED_WARNING: &str = "This RustDesk ID has not connected to this server yet, so \
                                    the server cannot tell which device started this login. \
                                    If your RustDesk client does not show this ID, do not \
                                    continue.";
const ID_TAKEN: &str = "This login cannot be completed: its RustDesk ID belongs to another \
                        device on this server. Nothing was signed in.";
/// The console page that completes a console login (`web/src/routes/login_.oidc.tsx`).
pub const CONSOLE_COMPLETION_PATH: &str = "/login/oidc";
/// The console page that confirms and completes a link (`web/src/routes/_app/me_.oidc-link.tsx`).
pub const CONSOLE_LINK_PATH: &str = "/me/oidc-link";

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

/// `POST`: completes the flow.
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

/// Whether the device of a live client flow may be signed in: `Ok(true)` when its id is
/// registered with its uuid, `Ok(false)` when the id is not registered yet; otherwise the
/// message of the page that refuses.
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

/// [`admit`] again at the confirming `POST`. Anything but a live client flow passes:
/// `complete_callback` gives its answer.
async fn recheck(state: &AppState, flow_state: Option<&str>) -> Result<(), &'static str> {
    match state.oidc.preview_callback(flow_state).await {
        Ok(CallbackPreview {
            target: PreviewTarget::Device(device),
            ..
        }) => admit(state, &device).await.map(drop),
        _ => Ok(()),
    }
}

/// A console login or link completes in the console itself (the browser that started it
/// holds the flow's binding cookie there): `303` to `<console><path>?<the IdP's query>`.
/// Only the query is copied; the target is fixed, so this is no open redirect.
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

/// What the confirmation page asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ask {
    /// The IdP did not sign the user in: tell the device to stop waiting.
    StopWaiting,
    /// Sign the device in; `registered`: its id is registered here with its uuid (else not
    /// registered yet, see [`UNREGISTERED_WARNING`]).
    SignIn { registered: bool },
}

/// The confirmation page's text, escaped: a question, notes under it, the button.
struct Confirmation {
    question: String,
    notes: Vec<String>,
    button: &'static str,
}

fn confirmation(provider: &str, device: &DeviceSummary, ask: Ask) -> Response {
    let parts = confirmation_parts(provider, device, ask);
    page::confirm(&parts.question, &parts.notes, parts.button)
}

/// The device is named by its RustDesk ID, first and in bold: the one thing the user can
/// check against their own client. Name and OS are whatever the device sent at
/// `/api/oidc/auth` (an attacker's device can call itself "Your work laptop"), so they are
/// only a note, labelled as unverified.
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

/// What the device said about itself, escaped and labelled; `None` when it said nothing.
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
    fn the_rustdesk_id_comes_first_and_the_self_description_is_labelled() {
        let shown = device(Some("Your work PC"), Some("linux"));

        let parts = confirmation_parts("corp", &shown, Ask::SignIn { registered: true });

        assert_eq!(
            parts.question,
            "Sign in RustDesk device <strong>123456789</strong> with corp?"
        );
        assert_eq!(parts.button, "Sign in");
        let reported = &parts.notes[0];
        assert!(
            reported.starts_with("Reported by the device itself"),
            "{reported}"
        );
        assert!(reported.contains("Your work PC") && reported.contains("linux"));
        assert!(parts.notes.last().unwrap().contains("RustDesk ID"));
    }

    #[test]
    fn an_unregistered_id_gets_the_plain_warning_instead() {
        let shown = device(Some("host"), None);

        let parts = confirmation_parts("corp", &shown, Ask::SignIn { registered: false });

        assert_eq!(parts.button, "Sign in");
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
        assert!(
            hostile
                .question
                .starts_with("&lt;i&gt;p&lt;/i&gt; did not sign you in.")
        );
        assert!(hostile.notes[0].contains("&lt;b&gt;x&lt;/b&gt;"));
        assert_eq!(hostile.button, "Stop waiting");
    }
}
