//! Bootstrap configuration structure (05-config §2.2). Defaults are the spec.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::pins::Pins;
use crate::types::{Cidr, Secret};

pub const DEFAULT_DATA_DIR: &str = "/var/lib/deskhaven";
pub const DEFAULT_CLIENT_API_PORT: u16 = 21114;
pub const DEFAULT_SIGNAL_PORT: u16 = 21116;
pub const DEFAULT_MAX_CONNECTIONS: u32 = 10;
pub const DEFAULT_LOG_LEVEL: &str = "info";

/// Everything needed before the database is reachable. Changes need a restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Bootstrap {
    /// Key pair, master key, SQLite database.
    pub data_dir: PathBuf,
    /// External base URL, e.g. "https://rd.example.com"; required for OIDC and email links.
    pub public_url: String,
    pub listen: Listen,
    pub signal: Signal,
    pub database: Database,
    pub security: Security,
    pub log: Log,
    /// Pinned runtime settings (dotted keys); interpreted by the settings registry.
    #[schemars(with = "std::collections::BTreeMap<String, serde_json::Value>")]
    pub settings: Pins,
}

impl Default for Bootstrap {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            public_url: String::new(),
            listen: Listen::default(),
            signal: Signal::default(),
            database: Database::default(),
            security: Security::default(),
            log: Log::default(),
            settings: Pins::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Listen {
    /// RustDesk client HTTP API (hbbs port - 2 by default).
    pub client_api: SocketAddr,
    /// Empty = serve admin console and admin API on the client_api listener.
    #[serde(with = "empty_as_none")]
    #[schemars(with = "String")]
    pub admin: Option<SocketAddr>,
    /// Redirect target for client_api "/" when admin is split.
    pub admin_public_url: String,
}

impl Listen {
    /// The separate admin listener, if any. An address equal to `client_api`
    /// means "not split".
    pub fn admin_addr(&self) -> Option<SocketAddr> {
        self.admin.filter(|addr| *addr != self.client_api)
    }
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            client_api: SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_CLIENT_API_PORT)),
            admin: None,
            admin_public_url: String::new(),
        }
    }
}

/// Embedded hbbs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Signal {
    /// TCP+UDP; NAT test uses port-1, WebSocket port+2.
    pub port: u16,
    /// Empty = {data_dir}/id_ed25519 (generated on first start).
    pub key_file: PathBuf,
    /// Run the embedded signal server (rendezvous, NAT test, WebSocket).
    pub enabled: bool,
    /// WebSocket listener; empty = all interfaces on port+2. E.g. "127.0.0.1:21118" for a
    /// reverse proxy on the same host.
    #[serde(with = "empty_as_none")]
    #[schemars(with = "String")]
    pub ws_listen: Option<SocketAddr>,
}

impl Default for Signal {
    fn default() -> Self {
        Self {
            port: DEFAULT_SIGNAL_PORT,
            key_file: PathBuf::new(),
            enabled: true,
            ws_listen: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Database {
    /// Empty = "sqlite://{data_dir}/deskhaven.db"; or "postgres://...". May embed credentials.
    pub url: Secret,
    /// Size of the database connection pool.
    pub max_connections: u32,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            url: Secret::default(),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Security {
    /// 32-byte base64; empty = {data_dir}/master.key (generated, 0600, only while the
    /// database holds no account).
    pub master_key: Secret,
    /// CIDRs whose X-Forwarded-For is honored; empty = trust none.
    pub trusted_proxies: Vec<Cidr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Log {
    /// tracing EnvFilter syntax.
    pub level: String,
    /// Log line format.
    pub format: LogFormat,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            level: DEFAULT_LOG_LEVEL.to_owned(),
            format: LogFormat::Pretty,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    #[default]
    Pretty,
    Json,
}

/// JSON Schema of the bootstrap file, for editors and docs.
pub fn json_schema() -> serde_json::Value {
    schemars::schema_for!(Bootstrap).to_value()
}

/// `""` <-> `None` for optional `FromStr` values.
mod empty_as_none {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer, de};

    pub fn serialize<T: Display, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.collect_str(v),
            None => s.serialize_str(""),
        }
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
    where
        T: FromStr<Err: Display>,
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(d)?;
        match raw.trim() {
            "" => Ok(None),
            s => s.parse().map(Some).map_err(de::Error::custom),
        }
    }
}
