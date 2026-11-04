//! `GET /me/devices`: the devices the signed-in user owns, with their online state. Read
//! only (ownership is assigned by administrators and `deskhaven deploy`). The owner filter
//! is the session's user, never a query value: the query accepts paging only.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::devices::DeviceQuery;
use deskhaven_core::state::AppState;
use serde::Serialize;
use utoipa::ToSchema;

use crate::devices::view::{DeviceStatusView, DeviceView};
use crate::session::ConsoleUser;
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// A device of the user. Everything but the id and the status is what the device reported
/// about itself. Carries neither the uuid, the key nor the address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MyDevice {
    /// The id clients connect to.
    pub rustdesk_id: String,
    pub hostname: Option<String>,
    /// The OS user running the client.
    pub username: Option<String>,
    pub os: Option<String>,
    /// The client version.
    pub version: Option<String>,
    pub status: DeviceStatusView,
    pub online: bool,
    pub last_seen_at: i64,
}

impl From<DeviceView> for MyDevice {
    fn from(d: DeviceView) -> Self {
        Self {
            rustdesk_id: d.rustdesk_id,
            hostname: d.hostname,
            username: d.username,
            os: d.os,
            version: d.version,
            status: d.status,
            online: d.online,
            last_seen_at: d.last_seen_at,
        }
    }
}

/// The devices owned by the user.
#[utoipa::path(
    get,
    path = "/me/devices",
    operation_id = "list_my_devices",
    tag = "me",
    security(("console_session" = ["self"])),
    params(PageQuery),
    responses((status = 200, description = "One page of the user's devices", body = Paginated<MyDevice>))
)]
pub async fn list(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ValidQuery(page): ValidQuery<PageQuery>,
) -> Result<Json<Paginated<MyDevice>>, AdminApiError> {
    let query = DeviceQuery {
        owner_user_id: Some(principal.user_id),
        page: page.into(),
        ..DeviceQuery::default()
    };
    let found = state.devices.page(&query).await?;
    let items = Paginated::from_page(found, |s| MyDevice::from(DeviceView::from(s)));
    Ok(Json(items))
}
