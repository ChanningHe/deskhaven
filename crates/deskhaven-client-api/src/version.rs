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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn serializes_version_and_source_url_only() {
        let Json(info) = version().await;

        let value = serde_json::to_value(&info).unwrap();

        assert_eq!(
            value,
            serde_json::json!({ "version": VERSION, "source_url": SOURCE_URL })
        );
    }
}
