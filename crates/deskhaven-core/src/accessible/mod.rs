//! What a signed-in client may list in its group panel (03 §4, 06 §4.2): users, devices and
//! device groups, each as a stably sorted page. The visibility scope is part of every query
//! (`scope`), never a filter over a fetched list, and a page is at most
//! [`MAX_PAGE_SIZE`](crate::pagination::MAX_PAGE_SIZE) rows.

mod device_groups;
mod peers;
mod scope;
mod users;

use sea_orm::{DatabaseConnection, DbErr, EntityTrait, FromQueryResult, PaginatorTrait, Select};

pub use deskhaven_db::entity::device::DeviceStatus;
pub use peers::AccessiblePeer;

use crate::pagination::{Page, PageRequest};

/// Why a listing failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AccessibleError {
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("accessible listing storage failed")]
    Database(#[from] DbErr),
}

/// Lists what a viewer (a user id) can see. Stateless and cheap to clone.
#[derive(Clone)]
pub struct AccessibleService {
    db: DatabaseConnection,
}

impl AccessibleService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

/// The requested page of `select`, with the total number of rows it matches.
async fn fetch_page<E>(
    db: &DatabaseConnection,
    select: Select<E>,
    page: PageRequest,
) -> Result<Page<E::Model>, AccessibleError>
where
    E: EntityTrait,
    E::Model: FromQueryResult + Send + Sync,
{
    let index = page.index().map_err(|problem| AccessibleError::Invalid {
        field: problem.field,
        reason: problem.reason,
    })?;
    let paginator = select.paginate(db, page.page_size);
    let total = paginator.num_items().await?;
    let items = paginator.fetch_page(index).await?;
    Ok(Page {
        items,
        total,
        page: page.page,
        page_size: page.page_size,
    })
}
