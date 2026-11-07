//! `POST /users`, `GET|PATCH|DELETE /users/{id}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::pagination::{MAX_PAGE_SIZE, PageRequest};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AccountEdit, Credential, NewUser, Password, ProfileUpdate};
use garde::Validate;
use serde::Deserialize;
use utoipa::ToSchema;

use super::rules::Acting;
use super::view::{ShownSecret, UserCreated, UserDetail, UserView};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, ConsoleUser, ErrorCode, RoleView, ValidJson, ValidPath,
};

type Problem = crate::ErrorEnvelope<'static>;

/// A new local account. Without `password` the server generates a temporary one, returns
/// it once and requires a change at first login.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewUserBody {
    /// 1-64 characters of `A-Z a-z 0-9 . _ @ -`, unique case-insensitively.
    #[garde(length(chars, min = 1, max = 64))]
    pub username: String,
    /// Blank: the username.
    #[serde(default)]
    #[garde(length(chars, max = 128))]
    pub display_name: String,
    #[garde(length(chars, max = 254))]
    pub email: Option<String>,
    #[serde(default = "default_role")]
    #[garde(skip)]
    pub role: RoleView,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub note: String,
    /// An initial password chosen by the administrator (checked against the policy).
    #[garde(length(chars, max = 256))]
    pub password: Option<String>,
    /// With `password`: require a change at first login (default true). A generated
    /// password always requires one.
    #[serde(default = "default_true")]
    #[garde(skip)]
    pub must_change_password: bool,
}

/// By hand: the chosen `password` prints as `<redacted>` (or `None`).
impl std::fmt::Debug for NewUserBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewUserBody")
            .field("username", &self.username)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .field("role", &self.role)
            .field("note", &self.note)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("must_change_password", &self.must_change_password)
            .finish()
    }
}

fn default_role() -> RoleView {
    RoleView::User
}

const fn default_true() -> bool {
    true
}

/// Changes to an account; absent fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UserPatch {
    /// Blank: the username.
    #[garde(length(chars, max = 128))]
    pub display_name: Option<String>,
    /// An empty string removes the address. A new address is unverified.
    #[garde(length(chars, max = 254))]
    pub email: Option<String>,
    /// Mark the address verified (or not) after checking it out of band. A verified
    /// address lets OIDC logins link the account by email (`auth.oidc.link_email_max_role`).
    /// `true` is its own step: never with `email`, and with `expected_email`.
    #[garde(skip)]
    pub email_verified: Option<bool>,
    /// With `email_verified: true`: the address that was checked. A different stored
    /// address (changed meanwhile) is 409 `conflict`, `details.reason: email_changed`.
    #[garde(length(chars, max = 254))]
    pub expected_email: Option<String>,
    #[garde(skip)]
    pub role: Option<RoleView>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
}

/// One account with its groups and usable sessions.
#[utoipa::path(
    get,
    path = "/users/{id}",
    tag = "users",
    operation_id = "get_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "The account", body = UserDetail),
        (status = 404, description = "No such user (`not_found`)", body = Problem),
    )
)]
pub async fn get_user(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<UserDetail>, AdminApiError> {
    let user = state.users.get(id).await?;
    let groups = state.groups.groups_of_user(id).await?;
    let recent = PageRequest {
        page: 1,
        page_size: MAX_PAGE_SIZE,
    };
    let sessions = state.sessions.list_for_user(id, recent).await?;
    let identities = state.oidc.identities_of(id).await?;
    Ok(Json(UserDetail {
        user: user.into(),
        groups: groups.into_iter().map(Into::into).collect(),
        identities: identities.into_iter().map(Into::into).collect(),
        sessions: sessions.items.into_iter().map(Into::into).collect(),
        sessions_total: sessions.total,
    }))
}

/// Creates a local account (audited `user.create`).
#[utoipa::path(
    post,
    path = "/users",
    tag = "users",
    operation_id = "create_user",
    security(("console_session" = ["admin"])),
    request_body = NewUserBody,
    responses(
        (status = 201, description = "Created; the generated password is shown only here", body = UserCreated),
        (status = 409, description = "Username or email taken (`conflict`, `details.field`)", body = Problem),
        (status = 422, description = "Invalid field or password policy (`validation_failed`)", body = Problem),
    )
)]
pub async fn create_user(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewUserBody>,
) -> Result<(StatusCode, Audited, Json<UserCreated>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let (password, generated) = match body.password {
        Some(chosen) => (Password::new(chosen), false),
        None => (state.users.temporary_password()?, true),
    };
    let shown = generated.then(|| ShownSecret::new(&password));
    let new = NewUser {
        display_name: body.display_name,
        email: body.email.filter(|e| !e.is_empty()),
        role: body.role.into(),
        must_change_password: generated || body.must_change_password,
        note: body.note,
        ..NewUser::new(body.username, Credential::Local { password })
    };
    let guard = acting.user_guard(&state.console_audit, "user.create", None);
    let user = state.users.create_guarded(new, guard).await?;
    let created = UserCreated {
        user: user.into(),
        temporary_password: shown,
    };
    Ok((StatusCode::CREATED, Audited, Json(created)))
}

/// Changes profile fields and the role in one transaction (audited `user.update`).
#[utoipa::path(
    patch,
    path = "/users/{id}",
    tag = "users",
    operation_id = "update_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    request_body = UserPatch,
    responses(
        (status = 200, description = "The updated account", body = UserView),
        (status = 403, description = "Owner accounts and the owner role need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "Email taken (`conflict`), the address changed since it was checked (`conflict`, `details.reason: email_changed`) or the last owner (`last_owner`)", body = Problem),
        (status = 422, description = "Invalid field (`validation_failed`)", body = Problem),
    )
)]
pub async fn update_user(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<UserPatch>,
) -> Result<(Audited, Json<UserView>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let expected = expected_email(&body)?;
    let edit = AccountEdit {
        profile: ProfileUpdate {
            display_name: body.display_name,
            email: body.email.map(|e| Some(e).filter(|e| !e.is_empty())),
            email_verified: body.email_verified,
            note: body.note,
            ..ProfileUpdate::default()
        },
        role: body.role.map(Into::into),
        status: None,
    };
    let guard = acting.update_guard(&state.console_audit, expected);
    let user = state.users.edit(id, edit, guard).await?;
    Ok((Audited, Json(user.into())))
}

/// Marking an address verified is its own step, about the address the administrator
/// checked: never with a new `email`, always with `expected_email`.
fn expected_email(body: &UserPatch) -> Result<Option<String>, AdminApiError> {
    if body.email_verified != Some(true) {
        return Ok(None);
    }
    if body.email.is_some() {
        return Err(invalid_field(
            "email_verified",
            "mark an address verified in its own request, after the change",
        ));
    }
    match body.expected_email.as_deref() {
        Some(expected) if !expected.is_empty() => Ok(Some(expected.to_owned())),
        _ => Err(invalid_field("expected_email", "required")),
    }
}

/// Deletes an account with its sessions, memberships and personal address book (audited
/// `user.delete`). Nobody deletes their own account.
#[utoipa::path(
    delete,
    path = "/users/{id}",
    tag = "users",
    operation_id = "delete_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "Own account (`self_delete`) or the last owner (`last_owner`)", body = Problem),
    )
)]
pub async fn delete_user(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    if id == acting.user_id {
        return Err(AdminApiError::new(
            ErrorCode::SelfDelete,
            "you cannot delete your own account",
        ));
    }
    let guard = acting.user_guard(&state.console_audit, "user.delete", None);
    state.users.delete_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
