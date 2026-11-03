//! Devices in the viewer's scope.

use std::collections::HashMap;

use deskhaven_db::entity::{device, device_group, user};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};

use super::scope::visible_devices;
use super::{AccessibleError, AccessibleService, DeviceStatus, fetch_page};
use crate::pagination::{Page, PageRequest};

/// A device in the group panel, with the names the panel shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessiblePeer {
    pub rustdesk_id: String,
    /// The administrator's `display_name`, else the reported hostname.
    pub device_name: Option<String>,
    /// The administrator's `display_username`, else the reported username.
    pub username: Option<String>,
    pub os: Option<String>,
    /// Username of the owner. `None` only if the owner vanished since the page was read:
    /// unowned devices are out of scope.
    pub owner: Option<String>,
    pub device_group: Option<String>,
    pub note: String,
    pub status: DeviceStatus,
}

impl AccessibleService {
    /// One page of the devices `viewer` can see (owned by themselves or a teammate), by
    /// RustDesk id; `status` keeps only devices in that state.
    pub async fn peers(
        &self,
        viewer: i64,
        status: Option<DeviceStatus>,
        page: PageRequest,
    ) -> Result<Page<AccessiblePeer>, AccessibleError> {
        let mut select = device::Entity::find().filter(visible_devices(viewer));
        if let Some(status) = status {
            select = select.filter(device::Column::Status.eq(status));
        }
        let select = select.order_by_asc(device::Column::RustdeskId);
        let found = fetch_page(&self.db, select, page).await?;
        let owners = owner_names(&self.db, &found.items).await?;
        let groups = group_names(&self.db, &found.items).await?;
        let items = found
            .items
            .into_iter()
            .map(|d| AccessiblePeer {
                owner: d.owner_user_id.and_then(|id| owners.get(&id).cloned()),
                device_group: d.device_group_id.and_then(|id| groups.get(&id).cloned()),
                rustdesk_id: d.rustdesk_id,
                device_name: preferred(d.display_name, d.hostname),
                username: preferred(d.display_username, d.username),
                os: d.os,
                note: d.note.unwrap_or_default(),
                status: d.status,
            })
            .collect();
        Ok(Page {
            items,
            total: found.total,
            page: found.page,
            page_size: found.page_size,
        })
    }
}

/// The administrator's name when it holds text, else what the device reported.
fn preferred(display: Option<String>, reported: Option<String>) -> Option<String> {
    display.filter(|name| !name.trim().is_empty()).or(reported)
}

/// Usernames of the owners of `devices`, by user id.
async fn owner_names(
    db: &DatabaseConnection,
    devices: &[device::Model],
) -> Result<HashMap<i64, String>, DbErr> {
    let ids: Vec<i64> = devices.iter().filter_map(|d| d.owner_user_id).collect();
    let rows = user::Entity::find()
        .select_only()
        .columns([user::Column::Id, user::Column::Username])
        .filter(user::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

/// Names of the device groups of `devices`, by group id.
async fn group_names(
    db: &DatabaseConnection,
    devices: &[device::Model],
) -> Result<HashMap<i64, String>, DbErr> {
    let ids: Vec<i64> = devices.iter().filter_map(|d| d.device_group_id).collect();
    let rows = device_group::Entity::find()
        .select_only()
        .columns([device_group::Column::Id, device_group::Column::Name])
        .filter(device_group::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}
