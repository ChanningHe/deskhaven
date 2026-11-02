//! `GET /users` and `GET /groups/{id}/members`: paginated, sortable, filterable account
//! lists (M3-03 list conventions: `page`, `page_size`, `sort=field|-field`, `q`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{UserQuery, UserSort, UserSortField};
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::view::{AuthSourceView, StatusView, UserView};
use crate::domain_error::invalid_field;
use crate::{AdminApiError, PageQuery, Paginated, RoleView, ValidPath, ValidQuery};

/// Longest accepted `q`.
const MAX_Q_CHARS: usize = 200;
const SORT_FIELDS: [(&str, UserSortField); 6] = [
    ("username", UserSortField::Username),
    ("display_name", UserSortField::DisplayName),
    ("role", UserSortField::Role),
    ("status", UserSortField::Status),
    ("created_at", UserSortField::CreatedAt),
    ("last_login_at", UserSortField::LastLoginAt),
];

/// Filters and order of a user list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UserFilter {
    /// `field` ascending or `-field` descending; then by id.
    #[garde(length(max = 32))]
    #[param(
        pattern = "^-?(username|display_name|role|status|created_at|last_login_at)$",
        example = "-created_at"
    )]
    pub sort: Option<String>,
    /// Substring of the username, email or display name (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub q: Option<String>,
    #[garde(skip)]
    pub role: Option<RoleView>,
    #[garde(skip)]
    pub status: Option<StatusView>,
    #[garde(skip)]
    pub auth_source: Option<AuthSourceView>,
    /// Members of this group only.
    #[garde(skip)]
    pub group: Option<i64>,
    /// Accounts outside this group (the candidates to add to it).
    #[garde(skip)]
    pub not_group: Option<i64>,
}

impl UserFilter {
    pub(crate) fn query(self, page: PageQuery) -> Result<UserQuery, AdminApiError> {
        Ok(UserQuery {
            status: self.status.map(Into::into),
            role: self.role.map(Into::into),
            auth_source: self.auth_source.map(Into::into),
            group_id: self.group,
            not_group_id: self.not_group,
            q: self
                .q
                .map(|q| q.trim().to_owned())
                .filter(|q| !q.is_empty()),
            sort: self.sort.as_deref().map(parse_sort).transpose()?,
            page: page.into(),
        })
    }
}

fn parse_sort(raw: &str) -> Result<UserSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    SORT_FIELDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, field)| UserSort { field, descending })
        .ok_or_else(|| invalid_field("sort", "unknown sort field"))
}

/// Accounts, filtered and paginated.
#[utoipa::path(
    get,
    path = "/users",
    tag = "users",
    operation_id = "list_users",
    security(("console_session" = ["admin"])),
    params(PageQuery, UserFilter),
    responses((status = 200, description = "One page of accounts", body = Paginated<UserView>))
)]
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<UserFilter>,
) -> Result<Json<Paginated<UserView>>, AdminApiError> {
    let found = state.users.list(&filter.query(page)?).await?;
    Ok(Json(Paginated::from_page(found, UserView::from)))
}

/// Members of a group (the same filters as the user list; `group` is the path's).
#[utoipa::path(
    get,
    path = "/groups/{id}/members",
    tag = "groups",
    operation_id = "list_group_members",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Group id"), PageQuery, UserFilter),
    responses(
        (status = 200, description = "One page of members", body = Paginated<UserView>),
        (status = 404, description = "No such group", body = crate::ErrorEnvelope<'static>),
    )
)]
pub async fn list_group_members(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<UserFilter>,
) -> Result<Json<Paginated<UserView>>, AdminApiError> {
    state.groups.get(id).await?;
    let filter = UserFilter {
        group: Some(id),
        ..filter
    };
    let found = state.users.list(&filter.query(page)?).await?;
    Ok(Json(Paginated::from_page(found, UserView::from)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_accepts_known_fields_in_both_directions() {
        let sort = parse_sort("-last_login_at").ok();
        assert_eq!(
            sort,
            Some(UserSort {
                field: UserSortField::LastLoginAt,
                descending: true
            })
        );
        assert!(parse_sort("username").is_ok());
        assert!(parse_sort("password_hash").is_err());
        assert!(parse_sort("--username").is_err());
    }
}
