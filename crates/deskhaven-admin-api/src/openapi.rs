//! The OpenAPI document of the admin API: the console's single source of API types
//! (08 §2, generated with orval). `cargo xtask gen-openapi` writes [`openapi_json`] of
//! the production routes to [`SPEC_PATH`]; CI fails when the committed file drifts.

use axum::http::Method;
use utoipa::openapi::path::{Operation, Parameter, ParameterBuilder, ParameterIn};
use utoipa::openapi::schema::{ObjectBuilder, Type};
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::openapi::{OpenApi as Document, Required};
use utoipa::{Modify, OpenApi};

use crate::error::ErrorEnvelope;
use crate::permission::{Permission, SESSION_SCHEME};
use crate::routes::{RouteError, error_response};
use crate::session::{SESSION_COOKIE, is_state_changing};

/// `session::CSRF_HEADER` in its documented spelling.
const CSRF_HEADER_NAME: &str = "X-Deskhaven-CSRF";

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
            state-changing request (not GET, HEAD or OPTIONS) must send the \
            `X-Deskhaven-CSRF` header with the value of the `__Host-deskhaven_csrf` cookie \
            (any non-empty value without a session); cross-site browser requests are refused. \
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

/// Documents what the gate adds to an operation (unless the handler already documents
/// that status): the required CSRF header on state-changing methods, and its failures.
pub(crate) fn add_gate_docs(operation: &mut Operation, method: &Method, permission: Permission) {
    let writes = is_state_changing(method);
    if writes {
        operation
            .parameters
            .get_or_insert_with(Vec::new)
            .push(csrf_parameter().into());
    }
    let mut failures = Vec::new();
    if permission != Permission::Public {
        failures.push(("401", "No valid console session (`unauthorized`)."));
        failures.push((
            "429",
            "Too many rejected sessions from this address (`rate_limited`).",
        ));
    }
    match (permission != Permission::Public, writes) {
        (true, true) => failures.push((
            "403",
            "Role not allowed (`forbidden`), or a missing or wrong CSRF header or a \
             cross-site request (`csrf_failed`).",
        )),
        (true, false) => failures.push(("403", "Role not allowed (`forbidden`).")),
        (false, true) => failures.push((
            "403",
            "A missing or wrong CSRF header or a cross-site request (`csrf_failed`).",
        )),
        (false, false) => {}
    }
    let responses = &mut operation.responses.responses;
    for (status, description) in failures {
        responses
            .entry(status.to_owned())
            .or_insert_with(|| error_response(description));
    }
}

fn csrf_parameter() -> Parameter {
    ParameterBuilder::new()
        .name(CSRF_HEADER_NAME)
        .parameter_in(ParameterIn::Header)
        .required(Required::True)
        .description(Some(
            "The `__Host-deskhaven_csrf` cookie's value; any non-empty value without a session.",
        ))
        .schema(Some(ObjectBuilder::new().schema_type(Type::String)))
        .build()
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
