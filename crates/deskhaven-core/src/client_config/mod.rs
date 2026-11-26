//! The formats are fixed by the official client; `tests/client_config` checks them against a
//! port of the client's decoders. Only public data lives here: never the private key.

mod filename;
mod resolve;
mod string;

pub use filename::WindowsNames;
pub use resolve::{ClientConfig, ConfigInput, HostSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The client rejects a configuration without an ID server.
    #[error("no ID server address: set public_url")]
    MissingHost,
    /// The client builds the text from UTF-16 code units, which only works for ASCII.
    #[error("configuration values must be ASCII")]
    NotAscii,
    #[error("failed to encode the configuration")]
    Encode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    /// Port 21116 is the client's default and omitted.
    pub host: String,
    /// Empty: whatever the ID server hands out.
    pub relay: String,
    pub api: String,
    pub key: String,
}

impl ServerConfig {
    /// What the mobile client's QR scan expects.
    pub fn qr_payload(&self) -> Result<String, ConfigError> {
        self.config_string().map(|s| format!("config={s}"))
    }

    pub(crate) fn values(&self) -> [&str; 4] {
        [&self.host, &self.relay, &self.api, &self.key]
    }
}
