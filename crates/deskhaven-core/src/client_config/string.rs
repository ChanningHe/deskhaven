//! Format: compact JSON `{host, relay, api, key}`, URL-safe base64 with padding, then reversed.
//! `--config` and the file name parser cut their input at every `--`; for ASCII text the
//! encoding never has two adjacent `-`, nor can the reversed string start with one.

use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE;
use serde::Serialize;

use super::{ConfigError, ServerConfig};

/// Field order of Dart's `Map` literal in `encode`.
#[derive(Serialize)]
struct Wire<'a> {
    host: &'a str,
    relay: &'a str,
    api: &'a str,
    key: &'a str,
}

impl ServerConfig {
    pub fn config_string(&self) -> Result<String, ConfigError> {
        if self.host.is_empty() {
            return Err(ConfigError::MissingHost);
        }
        if !self.values().iter().all(|v| v.is_ascii()) {
            return Err(ConfigError::NotAscii);
        }
        let json = serde_json::to_string(&Wire {
            host: &self.host,
            relay: &self.relay,
            api: &self.api,
            key: &self.key,
        })
        .map_err(|_| ConfigError::Encode)?;
        Ok(URL_SAFE.encode(json).chars().rev().collect())
    }
}
