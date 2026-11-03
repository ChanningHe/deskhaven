//! `GET /devices`: paginated, sortable, filterable device list (M3-03 list conventions:
//! `page`, `page_size`, `sort=field|-field`, `q`).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::devices::{DeviceQuery, DeviceSort, DeviceSortField};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::view::{DeviceStatusView, DeviceView};
use crate::domain_error::invalid_field;
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Longest accepted `q` and `os`.
const MAX_Q_CHARS: usize = 200;
const SORT_FIELDS: [(&str, DeviceSortField); 7] = [
    ("rustdesk_id", DeviceSortField::RustdeskId),
    ("hostname", DeviceSortField::Hostname),
    ("os", DeviceSortField::Os),
    ("version", DeviceSortField::Version),
    ("status", DeviceSortField::Status),
    ("last_seen_at", DeviceSortField::LastSeenAt),
    ("created_at", DeviceSortField::CreatedAt),
];

/// Filters and order of the device list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeviceFilter {
    /// `field` ascending or `-field` descending; then by id (default `rustdesk_id`).
    /// `last_seen_at` sorts by the stored value, which is flushed from memory in batches and
    /// can trail the `last_seen_at` shown for a device by one flush interval.
    #[garde(length(max = 32))]
    #[param(
        pattern = "^-?(rustdesk_id|hostname|os|version|status|last_seen_at|created_at)$",
        example = "-last_seen_at"
    )]
    pub sort: Option<String>,
    /// Substring of the RustDesk id, hostname, device username or note (case-insensitive,
    /// literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub q: Option<String>,
    /// Substring of the reported OS (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub os: Option<String>,
    /// Online now (`true`) or not (`false`).
    #[garde(skip)]
    pub online: Option<bool>,
    /// Devices owned by this user.
    #[garde(skip)]
    pub owner: Option<i64>,
    /// Devices in this device group.
    #[garde(skip)]
    pub device_group: Option<i64>,
    #[garde(skip)]
    pub status: Option<DeviceStatusView>,
}

impl DeviceFilter {
    fn query(self, page: PageQuery) -> Result<DeviceQuery, AdminApiError> {
        let text =
            |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        Ok(DeviceQuery {
            q: text(self.q),
            os: text(self.os),
            owner_user_id: self.owner,
            device_group_id: self.device_group,
            status: self.status.map(Into::into),
            online: self.online,
            sort: self.sort.as_deref().map(parse_sort).transpose()?,
            page: page.into(),
        })
    }
}

fn parse_sort(raw: &str) -> Result<DeviceSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    SORT_FIELDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, field)| DeviceSort { field, descending })
        .ok_or_else(|| invalid_field("sort", "unknown sort field"))
}

/// Devices, filtered and paginated, with their online status.
#[utoipa::path(
    get,
    path = "/devices",
    tag = "devices",
    operation_id = "list_devices",
    security(("console_session" = ["admin"])),
    params(PageQuery, DeviceFilter),
    responses((status = 200, description = "One page of devices", body = Paginated<DeviceView>))
)]
pub async fn list_devices(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<DeviceFilter>,
) -> Result<Json<Paginated<DeviceView>>, AdminApiError> {
    let found = state.devices.page(&filter.query(page)?).await?;
    Ok(Json(Paginated::from_page(found, DeviceView::from)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_accepts_known_fields_in_both_directions() {
        let sort = parse_sort("-last_seen_at").ok();
        assert_eq!(
            sort,
            Some(DeviceSort {
                field: DeviceSortField::LastSeenAt,
                descending: true
            })
        );
        assert!(parse_sort("hostname").is_ok());
        assert!(parse_sort("pk").is_err());
        assert!(parse_sort("--hostname").is_err());
    }
}
