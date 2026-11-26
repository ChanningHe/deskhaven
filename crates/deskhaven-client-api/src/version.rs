//! `GET /api/version`: what is running and where its source is. AGPL-3.0 §13
//! obliges a network service to offer the corresponding source to everyone who
//! interacts with it (06 §AGPL), so this takes no auth and cannot be disabled.
//!
//! The official client never calls it (no consumer at the tracked commit):
//! the shape is deskhaven's own, `{"version", "source_url"}`.

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
