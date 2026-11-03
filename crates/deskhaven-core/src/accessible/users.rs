//! Users in the viewer's scope.

use deskhaven_db::entity::user;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::scope::visible_users;
use super::{AccessibleError, AccessibleService, fetch_page};
use crate::pagination::{Page, PageRequest};
use crate::users::{Status, User};

impl AccessibleService {
    /// One page of the users `viewer` can see (themselves and their teammates), by username;
    /// `status` keeps only users in that state.
    pub async fn users(
        &self,
        viewer: i64,
        status: Option<Status>,
        page: PageRequest,
    ) -> Result<Page<User>, AccessibleError> {
        let mut select = user::Entity::find().filter(visible_users(viewer));
        if let Some(status) = status {
            select = select.filter(user::Column::Status.eq(status));
        }
        let select = select
            .order_by_asc(user::Column::UsernameNorm)
            .order_by_asc(user::Column::Id);
        let found = fetch_page(&self.db, select, page).await?;
        Ok(Page {
            items: found.items.into_iter().map(User::from).collect(),
            total: found.total,
            page: found.page,
            page_size: found.page_size,
        })
    }
}
