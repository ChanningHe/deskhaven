//! The OpenAPI document of the admin API: the console's single source of API types
//! (08 §2, generated with orval). `cargo xtask gen-openapi` writes [`openapi_json`] of
//! the production routes to [`SPEC_PATH`]; CI fails when the committed file drifts.

use utoipa::openapi::OpenApi as Document;
use utoipa::openapi::path::Operation;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};

use crate::error::ErrorEnvelope;
use crate::permission::{Permission, SESSION_SCHEME};
use crate::routes::{RouteError, error_response};
use crate::session::SESSION_COOKIE;

/// Committed location of the generated document, relative to the repository root.
pub const SPEC_PATH: &str = "crates/deskhaven-admin-api/openapi.json";

/// Name of the error body schema in `components.schemas`.
pub(crate) const ERROR_SCHEMA: &str = "ErrorEnvelope";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "deskhaven admin API",
        description = "Backs the deskhaven web console. Authenticated operations need the \
            console session cookie; their security requirement names the role they need \
            (self: any console user, admin: admin or owner, owner: owner). Every \
            state-changing request (not GET, HEAD or OPTIONS) must also send the \
            `X-Deskhaven-CSRF` header with the value of the `__Host-deskhaven_csrf` cookie. \
            Errors are `{\"error\": {\"code\", \"message\", \"details\"?}}` with a stable \
            `code`."
    ),
    components(schemas(ErrorEnvelope)),
    modifiers(&SessionScheme)
)]
struct ApiDoc;

struct SessionScheme;

impl Modify for SessionScheme {
    fn modify(&self, doc: &mut Document) {
        let scheme = ApiKey::Cookie(ApiKeyValue::with_description(
            SESSION_COOKIE,
            "Console session token (HttpOnly; set by the console login).",
        ));
        doc.components
            .get_or_insert_with(Default::default)
            .add_security_scheme(SESSION_SCHEME, SecurityScheme::ApiKey(scheme));
    }
}

/// The document without operations: info, security scheme and shared schemas.
pub(crate) fn base() -> Document {
    ApiDoc::openapi()
}

/// Documents the gate's own failures on an authenticated operation, unless the handler
/// already documents that status.
pub(crate) fn add_gate_responses(operation: &mut Operation, permission: Permission) {
    if permission == Permission::Public {
        return;
    }
    let gate_failures = [
        ("401", "No valid console session (`unauthorized`)."),
        (
            "403",
            "Role not allowed (`forbidden`), or a state-changing request without the \
             matching CSRF header (`csrf_failed`).",
        ),
        (
            "429",
            "Too many rejected sessions from this address (`rate_limited`).",
        ),
    ];
    let responses = &mut operation.responses.responses;
    for (status, description) in gate_failures {
        responses
            .entry(status.to_owned())
            .or_insert_with(|| error_response(description));
    }
}

/// The production routes' document, as committed: pretty JSON with a final newline.
pub fn openapi_json() -> Result<String, OpenApiError> {
    let doc = crate::api_routes()?.openapi();
    let mut json = doc.to_pretty_json().map_err(OpenApiError::Serialize)?;
    json.push('\n');
    Ok(json)
}

#[derive(Debug, thiserror::Error)]
pub enum OpenApiError {
    #[error(transparent)]
    Route(#[from] RouteError),
    #[error("cannot serialize the OpenAPI document")]
    Serialize(#[source] serde_json::Error),
}
