//! First-run setup (08 §3.2 `/setup`, M3-02) over `deskhaven_core::setup`:
//!
//! | method | path | permission | what |
//! |---|---|---|---|
//! | `GET` | `/setup` | public | whether the instance still needs its owner, and `public_url` |
//! | `POST` | `/setup` | public | the setup link's token and the owner account; signs the owner in |
//!
//! `POST` is per-IP limited (the token has 256 bits; the limit only bounds the work an
//! address can cause), requires a secure transport (it sets the session cookies) and
//! refuses with 409 `setup_completed` once an owner exists. The token check (constant
//! time), the single use, the owner creation and its audit row (`setup.complete`, actor =
//! the new owner via `Audit::actor`: a public route has no session to attribute it to)
//! are one transaction in the core service. A bad, used or expired token: 403
//! `invalid_setup_token` (not 401: there is no session to lose).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use deskhaven_core::console_audit::Actor;
use deskhaven_core::setup::{NewOwner, SetupError};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Password, PasswordError, UserError};
use deskhaven_http::X_REQUEST_ID;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{BUSY_RETRY_AFTER, LoginResponse, signed_in, user_agent};
use crate::routes::{AdminRoutes, RouteError};
use crate::session::Client;
use crate::transport::SecureTransport;
use crate::{AdminApiError, ErrorCode, ValidJson};

/// Longest accepted token, name or password input (characters).
const MAX_INPUT: usize = 1024;

pub(crate) fn routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new().routes(routes!(status, complete))
}

/// Whether the first-run setup is still open.
#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatus {
    /// No owner exists yet.
    pub required: bool,
    /// The configured `public_url` (bootstrap configuration, changed by a restart); `null`
    /// when unset (OIDC and e-mail links need it).
    pub public_url: Option<String>,
}

/// The setup link's token and the owner to create. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    /// The `token` of the setup link (`/setup#token=…`).
    #[garde(length(chars, min = 1, max = MAX_INPUT))]
    pub token: String,
    #[garde(length(chars, min = 1, max = MAX_INPUT))]
    pub username: String,
    /// Blank: the username.
    #[garde(inner(length(chars, max = MAX_INPUT)))]
    pub display_name: Option<String>,
    #[garde(inner(length(chars, max = MAX_INPUT)))]
    pub email: Option<String>,
    #[garde(length(chars, min = 1, max = MAX_INPUT))]
    #[schema(format = Password)]
    pub password: String,
}

/// What the audit row shows of the new owner.
#[derive(Serialize)]
struct OwnerSnapshot<'a> {
    username: &'a str,
    display_name: &'a str,
    email: Option<&'a str>,
    role: &'static str,
}

/// The first-run setup state.
#[utoipa::path(
    get,
    path = "/setup",
    operation_id = "get_setup_status",
    tag = "setup",
    security(()),
    responses((status = 200, description = "Setup state", body = SetupStatus))
)]
pub async fn status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SetupStatus>, AdminApiError> {
    let required = state
        .setup
        .is_required()
        .await
        .map_err(AdminApiError::internal)?;
    let public_url = Some(state.public_url.clone()).filter(|url| !url.is_empty());
    Ok(Json(SetupStatus {
        required,
        public_url,
    }))
}

/// Creates the owner with the setup link's token and signs them in.
#[utoipa::path(
    post,
    path = "/setup",
    operation_id = "complete_setup",
    tag = "setup",
    security(()),
    request_body = SetupRequest,
    responses(
        (status = 200, description = "The owner exists and is signed in (cookies set)", body = LoginResponse),
        (status = 403, description = "Unknown, used or expired token (`invalid_setup_token`), plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "An owner already exists (`setup_completed`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Invalid account fields or password policy (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts from this address (`rate_limited`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn complete(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    headers: HeaderMap,
    ValidJson(request): ValidJson<SetupRequest>,
) -> Result<Response, AdminApiError> {
    state
        .login_throttle
        .ip()
        .check(client)
        .map_err(AdminApiError::rate_limited)?;
    // Answered before the token is looked at (the transaction checks again).
    if !state
        .setup
        .is_required()
        .await
        .map_err(AdminApiError::internal)?
    {
        return Err(setup_error(SetupError::OwnerExists));
    }
    let ctx = AuditCtx {
        actor: Actor::System,
        ip: client.addr,
        request_id: headers
            .get(X_REQUEST_ID)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    };
    let owner = NewOwner {
        username: request.username,
        display_name: request.display_name.unwrap_or_default(),
        email: request.email.filter(|e| !e.trim().is_empty()),
        password: Password::new(request.password),
    };
    let done = state
        .setup
        .complete(&request.token, owner, |user| {
            let snapshot = OwnerSnapshot {
                username: &user.username,
                display_name: &user.display_name,
                email: user.email.as_deref(),
                role: "owner",
            };
            ctx.entry(
                Audit::new("setup.complete")
                    .actor(Actor::User(user.id))
                    .target("user", user.id)
                    .after(&snapshot),
            )
        })
        .await
        .map_err(setup_error)?;
    tracing::info!(user_id = done.user.id, ip = %client.addr, "first-run setup completed");
    let (session, token) = state
        .sessions
        .create_console(
            done.user.id,
            &done.stamp,
            Some(client.addr),
            user_agent(&headers),
        )
        .await
        .map_err(AdminApiError::internal)?;
    let response = signed_in(&state, done.user, &session, &token);
    Ok((Audited, response).into_response())
}

fn setup_error(err: SetupError) -> AdminApiError {
    match err {
        SetupError::InvalidToken => AdminApiError::new(
            ErrorCode::InvalidSetupToken,
            "the setup link is invalid, used or expired",
        ),
        SetupError::OwnerExists => AdminApiError::new(
            ErrorCode::SetupCompleted,
            "the instance already has an owner",
        ),
        SetupError::User(user) => user_error(user),
        other => AdminApiError::internal(other),
    }
}

fn user_error(err: UserError) -> AdminApiError {
    let field = |field: &str, problem: String| {
        AdminApiError::validation_failed("invalid owner account")
            .with_details(json!({ "fields": { field: [problem] } }))
    };
    match err {
        UserError::PolicyViolation { reason } => field("password", reason.to_string()),
        UserError::Invalid {
            field: name,
            reason,
        } => field(name, reason),
        UserError::Conflict { field: name } => {
            AdminApiError::conflict(format!("{name} is already taken"))
        }
        UserError::Password(PasswordError::Busy) => AdminApiError::unavailable(BUSY_RETRY_AFTER),
        other => AdminApiError::internal(other),
    }
}
