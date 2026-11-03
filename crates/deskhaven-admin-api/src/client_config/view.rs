//! Wire type of the client configuration endpoint.

use deskhaven_core::client_config::{ClientConfig, ConfigError, HostSource, ServerConfig};
use serde::Serialize;
use utoipa::ToSchema;

use crate::account::{QrCode, qr_code};

/// Where the ID server host came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostSourceView {
    /// The configured `public_url`.
    PublicUrl,
    /// The host the console was opened with; clients may not reach it.
    RequestHost,
    /// Neither: no ID server address.
    Unknown,
}

impl From<HostSource> for HostSourceView {
    fn from(source: HostSource) -> Self {
        match source {
            HostSource::PublicUrl => Self::PublicUrl,
            HostSource::RequestHost => Self::RequestHost,
            HostSource::Unknown => Self::Unknown,
        }
    }
}

/// What a RustDesk client needs to use this server. Public data only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ClientConfigView {
    /// ID server, `host[:port]` (the default port 21116 is left out); empty when unknown.
    pub id_server: String,
    /// API server base URL (`public_url`); empty when unset.
    pub api_server: String,
    /// The hbbs public key (base64), the clients' "Key".
    pub public_key: String,
    /// Every configured relay (`signal.relay_servers`) as `host:port`, for information: the
    /// configuration string leaves the relay out, the ID server hands relays out.
    pub relay_servers: Vec<String>,
    pub host_source: HostSourceView,
    /// The string for the client's import box and `rustdesk --config`; `null` without an
    /// ID server address.
    pub config_string: Option<String>,
    /// QR code of `config=<config_string>` for the mobile clients' scanner.
    pub qr: Option<QrCode>,
    /// `rustdesk-host=…,key=…,.exe`; `null` when a value cannot be in a Windows file name
    /// (an API URL, a port, an IPv6 address, a `/` in the key).
    pub windows_filename_plain: Option<String>,
    /// `rustdesk-licensed-<config string>.exe`; `null` without a config string.
    pub windows_filename_encoded: Option<String>,
}

impl ClientConfigView {
    pub(crate) fn from_config(config: ClientConfig) -> Self {
        let ClientConfig {
            server,
            host_source,
            relay_servers,
        } = config;
        let config_string = config_string_of(&server);
        let names = server.windows_names();
        let qr = server.qr_payload().ok().and_then(|text| qr_code(&text));
        Self {
            id_server: server.host,
            api_server: server.api,
            public_key: server.key,
            relay_servers,
            host_source: host_source.into(),
            config_string,
            qr,
            windows_filename_plain: names.plain,
            windows_filename_encoded: names.encoded,
        }
    }
}

/// No ID server address is an expected state; anything else is a bug worth a log line.
fn config_string_of(server: &ServerConfig) -> Option<String> {
    match server.config_string() {
        Ok(string) => Some(string),
        Err(ConfigError::MissingHost) => None,
        Err(error) => {
            tracing::warn!(%error, "client configuration string not available");
            None
        }
    }
}
