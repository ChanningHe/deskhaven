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
use deskhaven_signal::{AcceptLimits, ClientIpResolver, SignalConfig, WsHeaders};
use sodiumoxide::crypto::sign;

/// Call after `raise_nofile_limit`: the global connection caps follow the open-file soft limit.
pub fn base_config(
    signal: &Signal,
    security: &Security,
    key: &SignalKeyPair,
) -> anyhow::Result<SignalConfig> {
    let trusted = TrustedProxies::new(security.trusted_proxies.iter().map(Cidr::net));
    let open_files = open_file_limit();
    Ok(SignalConfig {
        bind: None,
        port: signal.port,
        ws_listen: signal.ws_listen,
        ws_client_ip: Arc::new(ProxyHeaders(trusted)),
        accept_limits: AcceptLimits::for_open_files(open_files),
        ..SignalConfig::new(secret_key(key)?)
    })
}

#[cfg(unix)]
fn open_file_limit() -> Option<u64> {
    rustix::process::getrlimit(rustix::process::Resource::Nofile).current
}

#[cfg(not(unix))]
fn open_file_limit() -> Option<u64> {
    None
}

#[derive(Debug)]
struct ProxyHeaders(TrustedProxies);

impl ClientIpResolver for ProxyHeaders {
    fn client_ip(&self, peer: IpAddr, headers: &WsHeaders) -> IpAddr {
        self.0.resolve(peer, headers).addr
    }

    fn is_trusted_proxy(&self, peer: IpAddr) -> bool {
        self.0.is_trusted(peer)
    }
}

/// sodiumoxide's layout (seed || public key); the copy is wiped on drop by sodiumoxide, the
/// scratch buffer by `Zeroizing`.
pub fn secret_key(key: &SignalKeyPair) -> anyhow::Result<sign::SecretKey> {
    sign::SecretKey::from_slice(key.keypair_bytes().as_slice())
        .context("the hbbs key pair does not have the libsodium secret key length")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalSettings {
    pub servers: Vec<String>,
    pub always_use_relay: bool,
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

    pub fn apply_to(&self, base: &SignalConfig) -> SignalConfig {
        SignalConfig {
            relay_servers: self.servers.clone(),
            always_use_relay: self.always_use_relay,
            require_kx_v1: self.require_kx_v1,
            ..base.clone()
        }
    }
}
