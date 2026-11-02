//! `SignalConfig` from the bootstrap configuration, the hbbs key pair and the settings.

use anyhow::Context;
use deskhaven_config::bootstrap::Signal;
use deskhaven_config::settings::{SIGNAL_ALWAYS_USE_RELAY, SIGNAL_RELAY_SERVERS, Snapshot};
use deskhaven_core::keys::SignalKeyPair;
use deskhaven_signal::SignalConfig;
use sodiumoxide::crypto::sign;

/// The server configuration of `[signal]`, before relay settings: upstream defaults, all
/// interfaces, the configured port and the hbbs key.
pub fn base_config(signal: &Signal, key: &SignalKeyPair) -> anyhow::Result<SignalConfig> {
    Ok(SignalConfig {
        bind: None,
        port: signal.port,
        ..SignalConfig::new(secret_key(key)?)
    })
}

/// The pair in sodiumoxide's `sign::SecretKey` layout (seed || public key, as upstream's
/// `id_ed25519`). The copy is wiped on drop by sodiumoxide; the scratch buffer by `Zeroizing`.
pub fn secret_key(key: &SignalKeyPair) -> anyhow::Result<sign::SecretKey> {
    sign::SecretKey::from_slice(key.keypair_bytes().as_slice())
        .context("the hbbs key pair does not have the libsodium secret key length")
}

/// The runtime relay settings the signal server follows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelaySettings {
    /// `signal.relay_servers` as `host:port` strings.
    pub servers: Vec<String>,
    /// `signal.always_use_relay`.
    pub always_use_relay: bool,
}

impl RelaySettings {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            servers: snapshot
                .get(&SIGNAL_RELAY_SERVERS)
                .iter()
                .map(ToString::to_string)
                .collect(),
            always_use_relay: snapshot.get(&SIGNAL_ALWAYS_USE_RELAY),
        }
    }

    /// `base` with these settings as its initial relay configuration.
    pub fn apply_to(&self, base: &SignalConfig) -> SignalConfig {
        SignalConfig {
            relay_servers: self.servers.clone(),
            always_use_relay: self.always_use_relay,
            ..base.clone()
        }
    }
}
