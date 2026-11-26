//! AGPL-3.0 §13 obliges a network service to offer its source to everyone who interacts with
//! it, so this takes no auth and cannot be disabled.

use axum::Json;
pub use deskhaven_core::build::{SOURCE_URL, VERSION};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub source_url: &'static str,
}

pub async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: VERSION,
        source_url: SOURCE_URL,
    })
}
