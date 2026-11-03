//! Group panel payloads (03 §4), read by `flutter/lib/models/group_model.dart`. The user
//! list is a list of [`ClientUserPayload`](super::ClientUserPayload).

use serde::Deserialize;

/// One item of `/api/device-group/accessible`.
///
/// `flutter/lib/common/hbbs/hbbs.dart:288-294`: `name` defaults to `''`; the mirror requires
/// it, since a nameless group cannot be selected.
#[derive(Debug, Deserialize)]
pub struct DeviceGroupPayload {
    pub name: String,
}

/// One item of `/api/peers`.
///
/// `flutter/lib/common/hbbs/hbbs.dart:77-93`: every key defaults (`?? ''`), but a present
/// value of another type throws; `info` of another type is read as `{}`, the mirror
/// requires an object. `group_model.dart:260-272` keys peers by `id`, so the mirror
/// requires it too.
#[derive(Debug, Deserialize)]
pub struct PeerPayload {
    pub id: String,
    pub info: PeerInfo,
    #[serde(default)]
    pub status: Option<i64>,
    #[serde(default)]
    pub user: Option<String>,
    /// Becomes the peer's `loginName`, which the panel matches against the selected user's
    /// `name`.
    #[serde(default)]
    pub user_name: Option<String>,
    #[serde(default)]
    pub device_group_name: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `PeerPayload.info` (`hbbs.dart:95-128`, `toPeer`): `username`, `os` (the platform is the
/// part before ` / `) and `device_name` (the hostname).
#[derive(Debug, Deserialize)]
pub struct PeerInfo {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
}
