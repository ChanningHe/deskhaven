//! `SignalConfig` from the bootstrap configuration, the hbbs key pair and the settings.

use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Context;
use deskhaven_config::Cidr;
use deskhaven_config::bootstrap::{Security, Signal};
use deskhaven_config::settings::{
    SIGNAL_ALWAYS_USE_RELAY, SIGNAL_RELAY_SERVERS, SIGNAL_REQUIRE_KX_V1, Snapshot,
};
use deskhaven_core::keys::SignalKeyPair;
use deskhaven_http::TrustedProxies;
use deskhaven_signal::{ClientIpResolver, SignalConfig, WsHeaders};
use sodiumoxide::crypto::sign;

/// The server configuration of `[signal]`, before relay settings: upstream defaults, all
/// interfaces, the configured port, the hbbs key, and `security.trusted_proxies` for the
/// websocket port's proxy headers.
pub fn base_config(
    signal: &Signal,
    security: &Security,
    key: &SignalKeyPair,
) -> anyhow::Result<SignalConfig> {
    let trusted = TrustedProxies::new(security.trusted_proxies.iter().map(Cidr::net));
    Ok(SignalConfig {
        bind: None,
        port: signal.port,
        ws_client_ip: Arc::new(ProxyHeaders(trusted)),
        ..SignalConfig::new(secret_key(key)?)
    })
}

/// The HTTP API's real-IP rule (`X-Forwarded-For` from `security.trusted_proxies` only) for
/// the signal server's websocket port.
#[derive(Debug)]
struct ProxyHeaders(TrustedProxies);

impl ClientIpResolver for ProxyHeaders {
    fn client_ip(&self, peer: IpAddr, headers: &WsHeaders) -> IpAddr {
        self.0.resolve(peer, headers).addr
    }
}

/// The pair in sodiumoxide's `sign::SecretKey` layout (seed || public key, as upstream's
/// `id_ed25519`). The copy is wiped on drop by sodiumoxide; the scratch buffer by `Zeroizing`.
pub fn secret_key(key: &SignalKeyPair) -> anyhow::Result<sign::SecretKey> {
    sign::SecretKey::from_slice(key.keypair_bytes().as_slice())
        .context("the hbbs key pair does not have the libsodium secret key length")
}

/// The settings the running signal server follows (relays, KeyExchange policy).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalSettings {
    /// `signal.relay_servers` as `host:port` strings.
    pub servers: Vec<String>,
    /// `signal.always_use_relay`.
    pub always_use_relay: bool,
    /// `signal.require_kx_v1`.
    pub require_kx_v1: bool,
}

impl SignalSettings {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            servers: snapshot
                .get(&SIGNAL_RELAY_SERVERS)
                .iter()
                .map(ToString::to_string)
                .collect(),
            always_use_relay: snapshot.get(&SIGNAL_ALWAYS_USE_RELAY),
            require_kx_v1: snapshot.get(&SIGNAL_REQUIRE_KX_V1),
        }
    }

    /// `base` with these settings as its initial runtime configuration.
    pub fn apply_to(&self, base: &SignalConfig) -> SignalConfig {
        SignalConfig {
            relay_servers: self.servers.clone(),
            always_use_relay: self.always_use_relay,
            require_kx_v1: self.require_kx_v1,
            ..base.clone()
        }
    }
}
