use serde::Deserialize;

/// `name` defaults to `''` in the client; the mirror requires it, since a nameless group cannot
/// be selected.
#[derive(Debug, Deserialize)]
pub struct DeviceGroupPayload {
    pub name: String,
}

/// A present value of another type throws; `info` of another type reads as `{}`. The panel keys
/// peers by `id`, so the mirror requires it.
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

/// The platform is the part of `os` before ` / `.
#[derive(Debug, Deserialize)]
pub struct PeerInfo {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
}
