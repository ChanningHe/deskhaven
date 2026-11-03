//! The config string.
//!
//! Format (`flutter/lib/common.dart:2956-2966`, `ServerConfig.encode`): the compact JSON
//! `{"host":…,"relay":…,"api":…,"key":…}`, URL-safe base64 with padding, then reversed.
//! Decoders: `common.dart:2938-2952` (import box, QR) and `src/custom_server.rs:24-42` with
//! `src/core_main.rs:488-512` (`rustdesk --config`).
//!
//! `--config` and the file name parser cut their input at every `--`
//! (`custom_server.rs:94-98`). `-` is in the URL-safe alphabet but never twice in a row for
//! ASCII text: an ASCII byte has its top bit clear, which keeps the first sextet of every
//! triple at most 31 and the third at most 61, so two adjacent sextets of 62 cannot occur,
//! nor can the reversed string start with one. A test checks it anyway.

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
    /// The string for the import box, `rustdesk --config` and the QR code.
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
