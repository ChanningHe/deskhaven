//! `GET|POST /strategies`, `GET|PATCH|DELETE /strategies/{id}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::{
    NewStrategy, StrategyQuery, StrategySort, StrategySortField, StrategyUpdate,
};
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::Problem;
use super::audit::strategy_guard;
use super::view::{NewStrategyBody, StrategyPatch, StrategyView, check_options};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, PageQuery, Paginated, ValidJson, ValidPath, ValidQuery,
};

/// Filters and order of the strategy list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StrategyFilter {
    /// `name` or `updated_at`, `-` prefix for descending (default `name`).
    #[garde(length(max = 32))]
    #[param(pattern = "^-?(name|updated_at)$")]
    pub sort: Option<String>,
    /// Substring of the name (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

fn parse_sort(raw: &str) -> Result<StrategySort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    let field = match name {
        "name" => StrategySortField::Name,
        "updated_at" => StrategySortField::UpdatedAt,
        _ => return Err(invalid_field("sort", "unknown sort field")),
    };
    Ok(StrategySort { field, descending })
}

/// Strategies with their assignment counts.
#[utoipa::path(
    get,
    path = "/strategies",
    tag = "strategies",
    operation_id = "list_strategies",
    security(("console_session" = ["admin"])),
    params(PageQuery, StrategyFilter),
    responses((status = 200, description = "One page of strategies", body = Paginated<StrategyView>))
)]
pub async fn list_strategies(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<StrategyFilter>,
) -> Result<Json<Paginated<StrategyView>>, AdminApiError> {
    let query = StrategyQuery {
        q: filter
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        sort: filter.sort.as_deref().map(parse_sort).transpose()?,
        page: page.into(),
    };
    let found = state.strategies.page(&query).await?;
    Ok(Json(Paginated::from_page(found, StrategyView::from)))
}

/// One strategy.
#[utoipa::path(
    get,
    path = "/strategies/{id}",
    tag = "strategies",
    operation_id = "get_strategy",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id")),
    responses(
        (status = 200, description = "The strategy", body = StrategyView),
        (status = 404, description = "No such strategy", body = Problem),
    )
)]
pub async fn get_strategy(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<StrategyView>, AdminApiError> {
    Ok(Json(state.strategies.summary(id).await?.into()))
}

/// Creates a strategy (audited `strategy.create`). Option values reach every device it is
/// assigned to in clear.
#[utoipa::path(
    post,
    path = "/strategies",
    tag = "strategies",
    operation_id = "create_strategy",
    security(("console_session" = ["admin"])),
    request_body = NewStrategyBody,
    responses(
        (status = 201, description = "Created", body = StrategyView),
        (status = 409, description = "Name taken (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_strategy(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewStrategyBody>,
) -> Result<(StatusCode, Audited, Json<StrategyView>), AdminApiError> {
    check_options(body.options.iter().map(|(k, v)| (k, Some(v))))?;
    let new = NewStrategy {
        note: body.note,
        ..NewStrategy::new(body.name, body.options)
    };
    let guard = strategy_guard(&state, &ctx, "strategy.create");
    let created = state.strategies.create_guarded(new, guard).await?;
    let summary = state.strategies.summary(created.id).await?;
    Ok((StatusCode::CREATED, Audited, Json(summary.into())))
}

/// Renames a strategy, changes its note or patches its options (audited `strategy.update`;
/// an empty patch changes and records nothing). Strategies from the configuration file are
/// read-only.
#[utoipa::path(
    patch,
    path = "/strategies/{id}",
    tag = "strategies",
    operation_id = "update_strategy",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id")),
    request_body = StrategyPatch,
    responses(
        (status = 200, description = "The updated strategy", body = StrategyView),
        (status = 404, description = "No such strategy", body = Problem),
        (status = 409, description = "Name taken (`conflict`) or managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn update_strategy(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<StrategyPatch>,
) -> Result<(Audited, Json<StrategyView>), AdminApiError> {
    let options_patch = body.options.unwrap_or_default();
    check_options(options_patch.iter().map(|(k, v)| (k, v.as_ref())))?;
    let update = StrategyUpdate {
        name: body.name,
        note: body.note,
        options_patch,
        ..StrategyUpdate::default()
    };
    let guard = strategy_guard(&state, &ctx, "strategy.update");
    state.strategies.update_guarded(id, update, guard).await?;
    let summary = state.strategies.summary(id).await?;
    Ok((Audited, Json(summary.into())))
}

/// Deletes a strategy and its assignments (audited `strategy.delete`); the next heartbeat
/// of its devices deletes the options it set. Strategies from the configuration file are
/// read-only.
#[utoipa::path(
    delete,
    path = "/strategies/{id}",
    tag = "strategies",
    operation_id = "delete_strategy",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such strategy", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn delete_strategy(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = strategy_guard(&state, &ctx, "strategy.delete");
    state.strategies.delete_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
