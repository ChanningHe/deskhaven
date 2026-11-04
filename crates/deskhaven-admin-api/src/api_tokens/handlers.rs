//! `GET|POST /api-tokens`, `POST /api-tokens/{id}/revoke`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use deskhaven_core::api_tokens::{
    ApiTokenEntry, ApiTokenQuery, ApiTokenSort, ApiTokenSortField, NewApiToken, expiry_from_millis,
};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::rules::{may_revoke, token_guard};
use super::view::{ApiTokenCreated, ApiTokenView, NewApiTokenBody, ShownToken};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, ConsoleUser, PageQuery, Paginated, ValidJson, ValidPath,
    ValidQuery,
};

type Problem = crate::ErrorEnvelope<'static>;

/// Filters and order of the token list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TokenFilter {
    /// `name` or `created_at`, `-` prefix for descending (default `-created_at`).
    #[garde(length(max = 32))]
    #[param(pattern = "^-?(name|created_at)$")]
    pub sort: Option<String>,
    /// Substring of the name (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

fn parse_sort(raw: &str) -> Result<ApiTokenSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    let field = match name {
        "name" => ApiTokenSortField::Name,
        "created_at" => ApiTokenSortField::CreatedAt,
        _ => return Err(invalid_field("sort", "unknown sort field")),
    };
    Ok(ApiTokenSort { field, descending })
}

fn view(state: &AppState, entry: ApiTokenEntry) -> ApiTokenView {
    ApiTokenView::of(entry, state.clock.now_ms())
}

async fn entry_of(state: &AppState, id: i64) -> Result<ApiTokenEntry, AdminApiError> {
    let found = state.api_tokens.entry(id).await?;
    found.ok_or_else(|| AdminApiError::not_found("API token not found"))
}

/// API tokens, revoked and expired ones included (newest first by default).
#[utoipa::path(
    get,
    path = "/api-tokens",
    tag = "api-tokens",
    operation_id = "list_api_tokens",
    security(("console_session" = ["admin"])),
    params(PageQuery, TokenFilter),
    responses((status = 200, description = "One page of API tokens", body = Paginated<ApiTokenView>))
)]
pub async fn list_api_tokens(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<TokenFilter>,
) -> Result<Json<Paginated<ApiTokenView>>, AdminApiError> {
    let query = ApiTokenQuery {
        q: filter
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        sort: filter.sort.as_deref().map(parse_sort).transpose()?,
        page: page.into(),
    };
    let found = state.api_tokens.page(&query).await?;
    Ok(Json(Paginated::from_page(found, |e| view(&state, e))))
}

/// Creates a token with the caller's authority and returns its secret, which is shown only
/// here (audited `api_token.create`; the secret is not in the audit row).
#[utoipa::path(
    post,
    path = "/api-tokens",
    tag = "api-tokens",
    operation_id = "create_api_token",
    security(("console_session" = ["admin"])),
    request_body = NewApiTokenBody,
    responses(
        (status = 201, description = "The token and its secret, shown only here", body = ApiTokenCreated),
        (status = 403, description = "The caller is no longer an administrator (`forbidden`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_api_token(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewApiTokenBody>,
) -> Result<
    (
        StatusCode,
        Audited,
        [(header::HeaderName, HeaderValue); 1],
        Json<ApiTokenCreated>,
    ),
    AdminApiError,
> {
    let expires_at = body
        .expires_at
        .map(|ms| expiry_from_millis(ms).ok_or_else(|| invalid_field("expires_at", "out of range")))
        .transpose()?;
    let new = NewApiToken {
        name: body.name,
        scopes: body.scopes.into_iter().map(Into::into).collect(),
        created_by: Some(principal.user_id),
        expires_at,
    };
    let guard = token_guard(&ctx, &state.console_audit, "api_token.create", None);
    let (entry, secret) = state.api_tokens.create_guarded(new, guard).await?;
    tracing::info!(
        token_id = entry.token.id,
        actor = principal.user_id,
        "API token created"
    );
    let created = ApiTokenCreated {
        token: view(&state, entry),
        secret: ShownToken::new(&secret),
    };
    let no_store = [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))];
    Ok((StatusCode::CREATED, Audited, no_store, Json(created)))
}

/// Revokes a token; it stops working on its next use. Revoking a revoked token changes
/// nothing. A token of an owner, or one without creator, needs an owner (`owner_required`).
/// Audited `api_token.revoke`.
#[utoipa::path(
    post,
    path = "/api-tokens/{id}/revoke",
    tag = "api-tokens",
    operation_id = "revoke_api_token",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "API token id")),
    responses(
        (status = 200, description = "The revoked token", body = ApiTokenView),
        (status = 403, description = "An owner's token needs an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such token", body = Problem),
    )
)]
pub async fn revoke_api_token(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<ApiTokenView>), AdminApiError> {
    let guard = token_guard(
        &ctx,
        &state.console_audit,
        "api_token.revoke",
        Some(may_revoke),
    );
    if !state.api_tokens.revoke_guarded(id, guard).await? {
        return Err(AdminApiError::not_found("API token not found"));
    }
    tracing::info!(
        token_id = id,
        actor = principal.user_id,
        "API token revoked"
    );
    Ok((Audited, Json(view(&state, entry_of(&state, id).await?))))
}
