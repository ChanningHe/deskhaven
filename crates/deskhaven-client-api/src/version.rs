//! `GET /api/version`: what is running and where its source is. AGPL-3.0 §13
//! obliges a network service to offer the corresponding source to everyone who
//! interacts with it (06 §AGPL), so this takes no auth and cannot be disabled.
//!
//! The official client never calls it (no consumer at the tracked commit):
//! the shape is deskhaven's own, `{"version", "source_url"}`.

use axum::Json;
use serde::Serialize;

/// The workspace version (every crate shares it).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Where the corresponding source of this build is offered. A constant on
/// purpose: a deployment must not be able to point its users elsewhere by
/// configuration; a fork changes it here when it changes the code.
pub const SOURCE_URL: &str = "https://git.homielab.cc/ChanningHe/deskhaven";

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

    #[test]
    fn source_url_is_an_absolute_https_url_without_trailing_slash() {
        assert!(SOURCE_URL.starts_with("https://"));
        assert!(!SOURCE_URL.ends_with('/'));
    }

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
