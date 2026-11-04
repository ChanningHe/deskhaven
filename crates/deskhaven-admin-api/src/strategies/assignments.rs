//! `GET|POST|DELETE /strategies/{id}/assignments`: the targets of a strategy. A target has
//! at most one strategy, so assigning replaces the strategy the target had.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;

use super::Problem;
use super::audit::assignment_guard;
use super::view::{AssignmentList, AssignmentView, TargetBody};
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath, ValidQuery};

/// The targets of a strategy, with their names.
#[utoipa::path(
    get,
    path = "/strategies/{id}/assignments",
    tag = "strategies",
    operation_id = "list_strategy_assignments",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id")),
    responses(
        (status = 200, description = "The targets", body = AssignmentList),
        (status = 404, description = "No such strategy", body = Problem),
    )
)]
pub async fn list_assignments(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<AssignmentList>, AdminApiError> {
    let targets = state.strategies.assigned_targets(id).await?;
    Ok(Json(AssignmentList {
        items: targets.into_iter().map(AssignmentView::from).collect(),
    }))
}

/// Assigns the strategy to a target, replacing the strategy the target had (audited
/// `strategy.assign`, with the replaced assignment as `before`). Assignments of a strategy
/// from the configuration file are read-only: neither it nor a target that has it can be
/// (re)assigned here.
#[utoipa::path(
    post,
    path = "/strategies/{id}/assignments",
    tag = "strategies",
    operation_id = "assign_strategy",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id")),
    request_body = TargetBody,
    responses(
        (status = 204, description = "Assigned"),
        (status = 404, description = "No such strategy", body = Problem),
        (status = 409, description = "The strategy, or the one the target has, is managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid or unknown target", body = Problem),
    )
)]
pub async fn assign_strategy(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<TargetBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let target = body.target()?;
    let guard = assignment_guard(&state, &ctx, "strategy.assign");
    state.strategies.assign_guarded(target, id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Removes the strategy from a target (audited `strategy.unassign`). Assignments of a
/// strategy from the configuration file are read-only.
#[utoipa::path(
    delete,
    path = "/strategies/{id}/assignments",
    tag = "strategies",
    operation_id = "unassign_strategy",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Strategy id"), TargetBody),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "The target does not have this strategy", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid target", body = Problem),
    )
)]
pub async fn unassign_strategy(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidQuery(query): ValidQuery<TargetBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let target = query.target()?;
    let guard = assignment_guard(&state, &ctx, "strategy.unassign");
    match state.strategies.unassign_guarded(target, id, guard).await? {
        true => Ok((Audited, StatusCode::NO_CONTENT)),
        false => Err(AdminApiError::not_found(
            "the target does not have this strategy",
        )),
    }
}
