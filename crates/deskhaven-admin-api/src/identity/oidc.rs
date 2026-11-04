//! `GET|POST /oidc-providers`, `GET|PATCH|DELETE /oidc-providers/{id}`. Admin routes.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;

use super::guard::provider_guard;
use super::oidc_view::{
    NewOidcProviderBody, OidcProviderList, OidcProviderPatch, OidcProviderView,
};
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Every provider, and the callback URL to register at the identity providers.
#[utoipa::path(
    get,
    path = "/oidc-providers",
    tag = "identity",
    operation_id = "list_oidc_provider_configs",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "All providers", body = OidcProviderList))
)]
pub async fn list_oidc_provider_configs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<OidcProviderList>, AdminApiError> {
    let providers = state.oidc.list_providers().await?;
    Ok(Json(OidcProviderList {
        callback_url: state.oidc.redirect_uri().map(ToString::to_string),
        items: providers.into_iter().map(Into::into).collect(),
    }))
}

/// One provider.
#[utoipa::path(
    get,
    path = "/oidc-providers/{id}",
    tag = "identity",
    operation_id = "get_oidc_provider",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Provider id")),
    responses(
        (status = 200, description = "The provider", body = OidcProviderView),
        (status = 404, description = "No such provider", body = Problem),
    )
)]
pub async fn get_oidc_provider(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<OidcProviderView>, AdminApiError> {
    Ok(Json(state.oidc.get_provider(id).await?.into()))
}

/// Creates a provider (audited `oidc_provider.create`, without the secret).
#[utoipa::path(
    post,
    path = "/oidc-providers",
    tag = "identity",
    operation_id = "create_oidc_provider",
    security(("console_session" = ["admin"])),
    request_body = NewOidcProviderBody,
    responses(
        (status = 201, description = "Created", body = OidcProviderView),
        (status = 409, description = "Name taken (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_oidc_provider(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewOidcProviderBody>,
) -> Result<(StatusCode, Audited, Json<OidcProviderView>), AdminApiError> {
    let guard = provider_guard(&state, &ctx, "oidc_provider.create");
    let created = state
        .oidc
        .create_provider_guarded(body.into(), guard)
        .await?;
    Ok((StatusCode::CREATED, Audited, Json(created.into())))
}

/// Changes a provider (audited `oidc_provider.update`). The issuer cannot change;
/// providers from the configuration file are read-only.
#[utoipa::path(
    patch,
    path = "/oidc-providers/{id}",
    tag = "identity",
    operation_id = "update_oidc_provider",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Provider id")),
    request_body = OidcProviderPatch,
    responses(
        (status = 200, description = "The updated provider", body = OidcProviderView),
        (status = 404, description = "No such provider", body = Problem),
        (status = 409, description = "Name taken (`conflict`) or managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid or unknown field (the issuer is fixed)", body = Problem),
    )
)]
pub async fn update_oidc_provider(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<OidcProviderPatch>,
) -> Result<(Audited, Json<OidcProviderView>), AdminApiError> {
    let guard = provider_guard(&state, &ctx, "oidc_provider.update");
    let updated = state
        .oidc
        .update_provider_guarded(id, body.into(), guard)
        .await?;
    Ok((Audited, Json(updated.into())))
}

/// Deletes a provider and its in-flight logins (audited `oidc_provider.delete`). Refused
/// while accounts are linked to it; providers from the configuration file are read-only.
#[utoipa::path(
    delete,
    path = "/oidc-providers/{id}",
    tag = "identity",
    operation_id = "delete_oidc_provider",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Provider id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such provider", body = Problem),
        (status = 409, description = "Accounts are linked to it (`conflict`) or managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn delete_oidc_provider(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = provider_guard(&state, &ctx, "oidc_provider.delete");
    state.oidc.delete_provider_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
