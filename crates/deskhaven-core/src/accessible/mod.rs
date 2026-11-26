//! The visibility scope is part of every query, never a filter over a fetched list.

mod device_groups;
mod peers;
mod scope;
mod users;

use sea_orm::{DatabaseConnection, DbErr, EntityTrait, FromQueryResult, PaginatorTrait, Select};

pub use deskhaven_db::entity::device::DeviceStatus;
pub use peers::AccessiblePeer;

use crate::pagination::{Page, PageRequest};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AccessibleError {
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("accessible listing storage failed")]
    Database(#[from] DbErr),
}

#[derive(Clone)]
pub struct AccessibleService {
    db: DatabaseConnection,
}

impl AccessibleService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

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
