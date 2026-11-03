//! What a RustDesk client needs to use this server: ID server, relay server, API server and
//! public key, and the ways to hand them over (M3-11, 05 §5):
//!
//! - the config string: `ServerConfig.encode` of the Flutter client, accepted by its
//!   import box, `rustdesk --config <string>` and (as `config=<string>`) the mobile QR scan;
//! - Windows file names the client reads its server from (`rustdesk-host=…,key=….exe` and
//!   `rustdesk-licensed-<string>.exe`).
//!
//! The formats are fixed by the official client (pinned commit 23e5bb4); the sources are
//! cited where each is implemented, and `tests/client_config` checks them against a port
//! of the client's decoders. Only public data lives here: never the private key.

mod filename;
mod resolve;
mod string;

pub use filename::WindowsNames;
pub use resolve::{ClientConfig, ConfigInput, HostSource};

/// Why no config string can be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The client rejects a configuration without an ID server (`common.dart:3560-3571`).
    #[error("no ID server address: set public_url")]
    MissingHost,
    /// The client builds the text from UTF-16 code units, which only works for ASCII
    /// (`common.dart:2962`).
    #[error("configuration values must be ASCII")]
    NotAscii,
    /// Serializing four strings failed; not expected.
    #[error("failed to encode the configuration")]
    Encode,
}

/// The four values of the client's "ID/Relay server" dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// ID server, `host[:port]` (port 21116 is the client's default and omitted).
    pub host: String,
    /// Relay server, `host[:port]`; empty = whatever the ID server hands out.
    pub relay: String,
    /// API server base URL, `http(s)://…` without a trailing slash; empty = none.
    pub api: String,
    /// The hbbs public key (base64).
    pub key: String,
}

impl ServerConfig {
    /// `config=<string>`, what the mobile client's QR scan expects (`scan_page.dart:152-157`).
    pub fn qr_payload(&self) -> Result<String, ConfigError> {
        self.config_string().map(|s| format!("config={s}"))
    }

    pub(crate) fn values(&self) -> [&str; 4] {
        [&self.host, &self.relay, &self.api, &self.key]
    }
}
