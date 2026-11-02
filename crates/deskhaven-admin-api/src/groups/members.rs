//! `POST /groups/{id}/members`, `DELETE /groups/{id}/members/{user_id}` (the member list
//! is `users::list::list_group_members`). Both are idempotent.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;

use super::guard::member_guard;
use super::view::AddMemberBody;
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Adds a user to the group; already a member is fine (audited `group.member_add`).
#[utoipa::path(
    post,
    path = "/groups/{id}/members",
    tag = "groups",
    operation_id = "add_group_member",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Group id")),
    request_body = AddMemberBody,
    responses(
        (status = 204, description = "The user is a member"),
        (status = 404, description = "No such group or user", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn add_member(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<AddMemberBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = member_guard(&state, &ctx, "group.member_add", id);
    state
        .groups
        .add_member_guarded(id, body.user_id, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Removes a user from the group; not a member is fine (audited `group.member_remove`).
#[utoipa::path(
    delete,
    path = "/groups/{id}/members/{user_id}",
    tag = "groups",
    operation_id = "remove_group_member",
    security(("console_session" = ["admin"])),
    params(
        ("id" = i64, Path, description = "Group id"),
        ("user_id" = i64, Path, description = "User id"),
    ),
    responses(
        (status = 204, description = "The user is not a member"),
        (status = 404, description = "No such group", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn remove_member(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath((id, user_id)): ValidPath<(i64, i64)>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = member_guard(&state, &ctx, "group.member_remove", id);
    state
        .groups
        .remove_member_guarded(id, user_id, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
