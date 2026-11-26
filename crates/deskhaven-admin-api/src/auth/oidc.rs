//! Only the browser that started a console flow holds its binding cookie, so a forwarded
//! authorization URL or a planted callback link does nothing and no confirmation page is
//! needed.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, CallbackReport, FLOW_TTL, Failure, OidcError,
};
use deskhaven_core::sessions::SessionError;
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::{LoginResponse, event, revoke_presented, signed_in, user_agent};
use crate::session::{self, Client, http_only_cookie};
use crate::transport::SecureTransport;
use crate::{AdminApiError, ErrorCode, ValidJson};

pub const OIDC_BINDING_COOKIE: &str = "__Host-deskhaven_oidc";
const MAX_FIELD: usize = 2048;

/// An OIDC login button of the console.
#[derive(Debug, Serialize, ToSchema)]
pub struct OidcProvider {
    /// The provider's name, the `provider` of `POST /auth/oidc/start`.
    pub name: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidcStartRequest {
    #[garde(length(chars, min = 1, max = MAX_FIELD))]
    pub provider: String,
}

/// Where to send the browser.
// Not `Debug`: the URL carries the flow's state.
#[derive(Serialize, ToSchema)]
pub struct OidcStart {
    /// The IdP's authorization URL.
    pub url: String,
}

/// The IdP's answer, as it arrived on the callback (`?state=&code=` or `?state=&error=`).
/// Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidcCompleteRequest {
    #[garde(length(chars, min = 1, max = MAX_FIELD))]
    pub state: String,
    #[garde(inner(length(chars, max = MAX_FIELD)))]
    pub code: Option<String>,
    #[garde(inner(length(chars, max = MAX_FIELD)))]
    pub error: Option<String>,
}

/// The enabled OIDC providers, as login buttons (empty without `public_url`).
#[utoipa::path(
    get,
    path = "/auth/oidc/providers",
    operation_id = "list_oidc_providers",
    tag = "auth",
    security(()),
    responses((status = 200, description = "Login buttons", body = Vec<OidcProvider>))
)]
pub async fn providers(State(state): State<Arc<AppState>>) -> Json<Vec<OidcProvider>> {
    let buttons = state.oidc.login_buttons().await;
    Json(
        buttons
            .iter()
            .map(|b| OidcProvider {
                name: b.name.clone(),
            })
            .collect(),
    )
}

/// Starts a console OIDC login (see the module docs).
#[utoipa::path(
    post,
    path = "/auth/oidc/start",
    operation_id = "start_oidc_login",
    tag = "auth",
    security(()),
    request_body = OidcStartRequest,
    responses(
        (status = 200, description = "The authorization URL; the binding cookie is set", body = OidcStart),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 404, description = "Unknown or disabled provider, or OIDC not configured (`not_found`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many logins from this address (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "The provider is unreachable (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn start(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    ValidJson(request): ValidJson<OidcStartRequest>,
) -> Result<Response, AdminApiError> {
    state
        .login_throttle
        .ip()
        .check(client)
        .map_err(AdminApiError::rate_limited)?;
    let started = state
        .oidc
        .begin_console(&request.provider)
        .await
        .map_err(start_error)?;
    let cookie = http_only_cookie(OIDC_BINDING_COOKIE, started.binding.expose(), FLOW_TTL);
    let body = Json(OidcStart {
        url: started.url.to_string(),
    });
    Ok(([(header::SET_COOKIE, cookie)], body).into_response())
}

fn start_error(err: OidcError) -> AdminApiError {
    match err {
        OidcError::NotConfigured | OidcError::UnknownProvider => {
            AdminApiError::not_found("unknown OIDC provider")
        }
        OidcError::Unavailable(_) => {
            AdminApiError::new(ErrorCode::Unavailable, "the OIDC provider is unavailable")
        }
        other => AdminApiError::internal(other),
    }
}

/// Completes a console OIDC login in the browser that started it (see the module docs).
#[utoipa::path(
    post,
    path = "/auth/oidc/complete",
    operation_id = "complete_oidc_login",
    tag = "auth",
    security(()),
    request_body = OidcCompleteRequest,
    responses(
        (status = 200, description = "Signed in (cookies set)", body = LoginResponse),
        (status = 403, description = "The login failed (`oidc_failed`, `details.reason`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
    )
)]
pub async fn complete(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    headers: HeaderMap,
    ValidJson(request): ValidJson<OidcCompleteRequest>,
) -> Result<Response, AdminApiError> {
    let params = CallbackParams {
        state: Some(&request.state),
        code: request.code.as_deref(),
        error: request.error.as_deref(),
    };
    let binding = session::cookie(&headers, OIDC_BINDING_COOKIE);
    let report = match state.oidc.complete_console(&params, binding).await {
        Ok(report) => report,
        Err(CallbackError::InvalidState) => return Err(failed("expired")),
        Err(err) => return Err(AdminApiError::internal(err)),
    };
    record(&state, client, &report).await;
    let login = report.outcome.map_err(|f| failed(reason(f)))?;
    let created = state
        .sessions
        .create_console(
            login.user.id,
            &login.stamp,
            Some(client.addr),
            user_agent(&headers),
        )
        .await;
    let (session, token) = match created {
        Ok(created) => created,
        // Disabled, deleted or changed since the callback resolved it.
        Err(SessionError::StaleCredential) => return Err(failed("disabled")),
        Err(err) => return Err(AdminApiError::internal(err)),
    };
    revoke_presented(&state, &headers).await;
    let clear = clear_binding_cookie();
    let signed = signed_in(&state, login.user, &session, &token);
    Ok((AppendHeaders([(header::SET_COOKIE, clear)]), signed).into_response())
}

pub(crate) fn clear_binding_cookie() -> HeaderValue {
    http_only_cookie(OIDC_BINDING_COOKIE, "", Duration::ZERO)
}

fn failed(reason: &'static str) -> AdminApiError {
    AdminApiError::new(ErrorCode::OidcFailed, "the OIDC login failed")
        .with_details(json!({ "reason": reason }))
}

const fn reason(failure: Failure) -> &'static str {
    match failure {
        Failure::Denied => "denied",
        Failure::Unavailable => "unavailable",
        Failure::InvalidToken => "invalid_token",
        Failure::NoLinkedAccount => "no_linked_account",
        Failure::Disabled => "disabled",
        Failure::Unverified => "unverified",
        Failure::ProvisioningFailed => "provisioning_failed",
    }
}

async fn record(state: &AppState, client: ClientAddr, report: &CallbackReport) {
    let attempted = match &report.subject {
        Some(subject) => format!("{}:{subject}", report.provider),
        None => report.provider.clone(),
    };
    let (user_id, result) = match &report.outcome {
        Ok(login) => (Some(login.user.id), LoginResult::Success),
        Err(failure) => (None, LoginResult::from(*failure)),
    };
    event::record(
        state,
        client,
        &attempted,
        LoginMethod::Oidc,
        user_id,
        result,
    )
    .await;
}
