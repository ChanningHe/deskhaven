//! The group panel (03-client-api-contract §4), read by `flutter/lib/models/group_model.dart`:
//! `GET /api/users`, `GET /api/peers` and `GET /api/device-group/accessible`. Every endpoint
//! needs a client bearer token and lists only what the caller may see (06 §4.2, enforced by
//! the queries of `deskhaven_core::accessible`); a role changes nothing.
//!
//! | case | status | body |
//! |---|---|---|
//! | page | 200 | `{total, data}`; `total` counts the whole visible set |
//! | bad `current` / `pageSize` (0, over 200) or `status` | 400 | `{"error": ...}` |
//! | missing or invalid token | 401 | `{"error": "Unauthorized"}` (the client resets its login) |
//!
//! `accessible` is sent empty by the client and carries no meaning here: the list is always
//! the accessible set.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{RawQuery, State};
use axum::routing::get;
use deskhaven_core::accessible::{
    AccessibleError, AccessiblePeer, AccessibleService, DeviceStatus,
};
use deskhaven_core::state::AppState;
use deskhaven_core::users::Status;
use serde::Serialize;

use crate::query::{self, PageReply};
use crate::user_payload::UserPayload;
use crate::{BearerUser, ClientApiError};

/// The three routes (merged into the client API router).
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/users", get(users))
        .route("/api/peers", get(peers))
        .route("/api/device-group/accessible", get(device_groups))
}

/// `PeerPayload.fromJson` (`flutter/lib/common/hbbs/hbbs.dart:77-93`).
#[derive(Serialize)]
struct PeerReply {
    id: String,
    info: PeerInfoReply,
    /// 1 active, 0 disabled.
    status: u8,
    /// Owner's username.
    user: String,
    /// Owner's username again: the panel matches it against the selected user's `name`.
    user_name: String,
    device_group_name: String,
    note: String,
}

/// `PeerPayload.toPeer` (`hbbs.dart:95-128`) reads `username`, `os` and `device_name`.
#[derive(Serialize)]
struct PeerInfoReply {
    username: String,
    os: String,
    device_name: String,
}

#[derive(Serialize)]
struct DeviceGroupReply {
    name: String,
}

impl From<AccessiblePeer> for PeerReply {
    fn from(peer: AccessiblePeer) -> Self {
        let owner = peer.owner.unwrap_or_default();
        Self {
            id: peer.rustdesk_id,
            info: PeerInfoReply {
                username: peer.username.unwrap_or_default(),
                os: peer.os.unwrap_or_default(),
                device_name: peer.hostname.unwrap_or_default(),
            },
            status: match peer.status {
                DeviceStatus::Active => 1,
                DeviceStatus::Disabled => 0,
            },
            user: owner.clone(),
            user_name: owner,
            device_group_name: peer.device_group.unwrap_or_default(),
            note: peer.note,
        }
    }
}

fn service(state: &AppState) -> AccessibleService {
    AccessibleService::new(state.db.clone())
}

fn list_error(err: AccessibleError) -> ClientApiError {
    match err {
        AccessibleError::Invalid { field, reason } => {
            ClientApiError::BadRequest(format!("Invalid {field}: {reason}").into())
        }
        other => ClientApiError::internal(other),
    }
}

fn bad_status() -> ClientApiError {
    ClientApiError::BadRequest("Invalid status".into())
}

/// `status` of `/api/users`: 1, 0 or -1.
fn user_status(raw: Option<i8>) -> Result<Option<Status>, ClientApiError> {
    raw.map(|n| match n {
        1 => Ok(Status::Active),
        0 => Ok(Status::Disabled),
        -1 => Ok(Status::Unverified),
        _ => Err(bad_status()),
    })
    .transpose()
}

/// `status` of `/api/peers`: 1 (active) or 0 (disabled).
fn device_status(raw: Option<i8>) -> Result<Option<DeviceStatus>, ClientApiError> {
    raw.map(|n| match n {
        1 => Ok(DeviceStatus::Active),
        0 => Ok(DeviceStatus::Disabled),
        _ => Err(bad_status()),
    })
    .transpose()
}

/// The caller and the users sharing a group with them.
async fn users(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<PageReply<UserPayload>>, ClientApiError> {
    let page = query::page(raw.as_deref())?;
    let status = user_status(query::status(raw.as_deref())?)?;
    let found = service(&state)
        .users(principal.user_id, status, page)
        .await
        .map_err(list_error)?;
    Ok(Json(PageReply {
        total: found.total,
        data: found.items.iter().map(UserPayload::from).collect(),
    }))
}

/// The devices of the caller and of the users sharing a group with them.
async fn peers(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<PageReply<PeerReply>>, ClientApiError> {
    let page = query::page(raw.as_deref())?;
    let status = device_status(query::status(raw.as_deref())?)?;
    let found = service(&state)
        .peers(principal.user_id, status, page)
        .await
        .map_err(list_error)?;
    Ok(Json(PageReply {
        total: found.total,
        data: found.items.into_iter().map(PeerReply::from).collect(),
    }))
}

/// The device groups holding a device the caller can see.
async fn device_groups(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<PageReply<DeviceGroupReply>>, ClientApiError> {
    let page = query::page(raw.as_deref())?;
    let found = service(&state)
        .device_groups(principal.user_id, page)
        .await
        .map_err(list_error)?;
    Ok(Json(PageReply {
        total: found.total,
        data: found
            .items
            .into_iter()
            .map(|name| DeviceGroupReply { name })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_status_takes_the_three_client_values() {
        assert_eq!(user_status(None).unwrap(), None);
        assert_eq!(user_status(Some(1)).unwrap(), Some(Status::Active));
        assert_eq!(user_status(Some(0)).unwrap(), Some(Status::Disabled));
        assert_eq!(user_status(Some(-1)).unwrap(), Some(Status::Unverified));
        assert!(user_status(Some(2)).is_err());
    }

    #[test]
    fn device_status_takes_one_and_zero_only() {
        assert_eq!(device_status(Some(1)).unwrap(), Some(DeviceStatus::Active));
        assert_eq!(
            device_status(Some(0)).unwrap(),
            Some(DeviceStatus::Disabled)
        );
        assert!(device_status(Some(-1)).is_err());
    }
}
