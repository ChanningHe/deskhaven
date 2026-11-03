//! Device groups in the viewer's scope.

use deskhaven_db::entity::{device, device_group};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait};

use super::scope::visible_devices;
use super::{AccessibleError, AccessibleService, fetch_page};
use crate::pagination::{Page, PageRequest};

impl AccessibleService {
    /// One page of the names of the device groups that hold a device `viewer` can see, by
    /// name. A group none of whose devices is visible does not appear.
    pub async fn device_groups(
        &self,
        viewer: i64,
        page: PageRequest,
    ) -> Result<Page<String>, AccessibleError> {
        let used = device::Entity::find()
            .select_only()
            .column(device::Column::DeviceGroupId)
            .filter(visible_devices(viewer))
            .filter(device::Column::DeviceGroupId.is_not_null())
            .into_query();
        let select = device_group::Entity::find()
            .filter(device_group::Column::Id.in_subquery(used))
            .order_by_asc(device_group::Column::Name)
            .order_by_asc(device_group::Column::Id);
        let found = fetch_page(&self.db, select, page).await?;
        Ok(Page {
            items: found.items.into_iter().map(|g| g.name).collect(),
            total: found.total,
            page: found.page,
            page_size: found.page_size,
        })
    }
}
