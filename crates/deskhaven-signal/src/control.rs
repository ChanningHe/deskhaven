//! Control-plane hooks (deskhaven code, not vendored; 07-hbbs-fork §3.2, §5): the decisions
//! upstream hard-codes and Pro takes from its control plane, behind the [`ControlPlane`] trait
//! injected into [`SignalServer::new`](crate::SignalServer::new). [`AllowAll`] reproduces the
//! OSS behavior.
//!
//! Wired today: [`ControlPlane::pick_relay`] (every relay choice) and the liveness
//! notifications [`ControlPlane::peer_online`] / [`ControlPlane::peer_offline`] (M2-12: they
//! feed deskhaven's device presence table; no-ops by default). The other hooks are defined
//! so M5 can implement them, but the server does not call them yet; their insertion points
//! are marked `ControlPlane hook (M5)` in `rendezvous_server.rs`.

use async_trait::async_trait;
use hbb_common::rendezvous_proto::{
    ConnType, ControlPermissions, HttpProxyRequest, HttpProxyResponse,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicUsize, Ordering},
};

/// Answer to a RegisterPk that passed the protocol checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterVerdict {
    /// Register (upstream behavior).
    Ok,
    /// The device must be deployed first (`RegisterPkResponse::NOT_DEPLOYED`).
    NotDeployed,
    /// Refuse the id (`RegisterPkResponse::UUID_MISMATCH`).
    UuidMismatch,
}

/// A RegisterPk as seen by [`ControlPlane::register_pk`].
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RegisterInfo<'a> {
    pub id: &'a str,
    pub uuid: &'a [u8],
    pub pk: &'a [u8],
    pub ip: IpAddr,
    pub no_register_device: bool,
}

/// What a connect request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnKind {
    /// `PunchHoleRequest`.
    Punch,
    /// `RequestRelay`.
    Relay,
}

/// A connect request as seen by [`ControlPlane::authorize`]. It names no initiator id: the
/// protocol has none, identity can only come from `token`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ConnectReq<'a> {
    pub kind: ConnKind,
    /// The id of the peer to connect to.
    pub target: &'a str,
    pub token: &'a str,
    pub switch_code: &'a str,
    pub conn_type: ConnType,
    pub from: SocketAddr,
    /// Whether the request arrived on a KeyExchange-secured connection.
    pub secured: bool,
}

/// What an allowed connection carries to the controlled side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grant {
    /// Injected into PunchHole / FetchLocalAddr / RequestRelay; `None` leaves the peer's own.
    pub permissions: Option<ControlPermissions>,
    /// `conn_audit_ref` for the controlled side's audit log; empty for none.
    pub audit_ref: String,
    /// `feedback` for the controller (non-zero asks it to keep a HealthCheck connection).
    pub feedback: i32,
}

/// Decision of [`ControlPlane::authorize`].
#[derive(Clone, Debug, PartialEq)]
pub enum Authz {
    Allow(Grant),
    /// Refused; the text is shown to the controller as is.
    Deny(String),
}

/// Policy decisions of the signal server. Implementations must be cheap: they run on the
/// request paths.
#[async_trait]
pub trait ControlPlane: Send + Sync + 'static {
    /// Admission of a RegisterPk. Called inline on the UDP loop: must answer from memory,
    /// never await I/O. Not called yet (M5).
    fn register_pk(&self, r: &RegisterInfo<'_>) -> RegisterVerdict;

    /// Admission of a punch-hole or relay request, on the connection's own task. Should answer
    /// from an in-memory snapshot; never a network round trip per request. Not called yet (M5).
    async fn authorize(&self, r: &ConnectReq<'_>) -> Authz;

    /// Answers an `HttpProxyRequest` received on a secured connection. Not called yet (M5).
    async fn http_proxy(&self, from: SocketAddr, req: HttpProxyRequest) -> HttpProxyResponse;

    /// Accepts a HealthCheck for `token`: `Some(keep_alive_secs)` keeps the connection open,
    /// `None` refuses. Not called yet (M5).
    async fn health_check(&self, token: &str, from: SocketAddr) -> Option<i32>;

    /// The relay to hand out to a session between `a` (controller) and `b` (controlled), chosen
    /// from `relays`: the configured relays that resolved and passed the latest health check,
    /// in configuration order. `None` hands out none (clients fall back to their own relay
    /// setting). Called on every relay choice; must not block.
    fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String>;

    /// Registered peer `id` showed it is online at `ip`, i.e. the server just refreshed its
    /// registration time (what OnlineRequest and punch holes compare with REG_TIMEOUT): a
    /// RegisterPeer from its known address, an accepted RegisterPk that changed its entry, or
    /// its TCP / websocket channel being bound or answering a heartbeat. Ids the server does
    /// not hold a key for never get here. Called inline on the UDP loop and connection tasks,
    /// for every registered peer every few seconds: must return at once (memory only, no
    /// awaiting). Default: ignored.
    fn peer_online(&self, _id: &str, _ip: IpAddr) {}

    /// The TCP / websocket channel of peer `id` closed while it was the peer's address: the
    /// server now treats the peer as offline until it registers again. Same constraints as
    /// [`peer_online`](Self::peer_online). Default: ignored.
    fn peer_offline(&self, _id: &str) {}
}

/// The OSS behavior: everything is allowed, relays rotate round-robin.
#[derive(Debug, Default)]
pub struct AllowAll {
    /// Upstream `ROTATION_RELAY_SERVER`, per instance instead of process-wide.
    rotation: AtomicUsize,
}

impl AllowAll {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Shown to clients that send an `HttpProxyRequest`; upstream ignores the message.
const HTTP_PROXY_UNSUPPORTED: &str = "HTTP proxy is not supported by this server";

#[async_trait]
impl ControlPlane for AllowAll {
    fn register_pk(&self, _: &RegisterInfo<'_>) -> RegisterVerdict {
        RegisterVerdict::Ok
    }

    async fn authorize(&self, _: &ConnectReq<'_>) -> Authz {
        Authz::Allow(Grant::default())
    }

    async fn http_proxy(&self, _: SocketAddr, _: HttpProxyRequest) -> HttpProxyResponse {
        HttpProxyResponse {
            error: HTTP_PROXY_UNSUPPORTED.to_owned(),
            ..Default::default()
        }
    }

    async fn health_check(&self, _: &str, _: SocketAddr) -> Option<i32> {
        None
    }

    /// Upstream `get_relay_server` (rs:959-967): none for an empty list, the only one, or the
    /// next one round-robin; the addresses are not used.
    fn pick_relay(&self, _: IpAddr, _: IpAddr, relays: &[String]) -> Option<String> {
        match relays {
            [] => None,
            [only] => Some(only.clone()),
            _ => {
                let i = self.rotation.fetch_add(1, Ordering::Relaxed) % relays.len();
                relays.get(i).cloned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    const IP: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const FROM: SocketAddr = SocketAddr::new(IP, 1);

    fn relays(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn allow_all_picks_none_from_an_empty_list() {
        assert_eq!(AllowAll::new().pick_relay(IP, IP, &[]), None);
    }

    #[test]
    fn allow_all_always_picks_the_only_relay() {
        let cp = AllowAll::new();
        let list = relays(&["r1:21117"]);
        for _ in 0..3 {
            assert_eq!(cp.pick_relay(IP, IP, &list).as_deref(), Some("r1:21117"));
        }
    }

    #[test]
    fn allow_all_rotates_round_robin_per_instance() {
        let (a, b) = (AllowAll::new(), AllowAll::new());
        let list = relays(&["r1", "r2", "r3"]);
        let picks: Vec<_> = (0..6)
            .map(|_| a.pick_relay(IP, IP, &list).unwrap())
            .collect();
        assert_eq!(picks, ["r1", "r2", "r3", "r1", "r2", "r3"]);
        assert_eq!(b.pick_relay(IP, IP, &list).as_deref(), Some("r1"));
    }

    #[tokio::test]
    async fn allow_all_admits_everything_and_offers_no_extras() {
        let cp = AllowAll::new();
        let reg = RegisterInfo {
            id: "123456789",
            uuid: b"uuid",
            pk: &[0; 32],
            ip: IP,
            no_register_device: false,
        };
        let req = ConnectReq {
            kind: ConnKind::Punch,
            target: "123456789",
            token: "",
            switch_code: "",
            conn_type: ConnType::DEFAULT_CONN,
            from: FROM,
            secured: false,
        };
        assert_eq!(cp.register_pk(&reg), RegisterVerdict::Ok);
        assert_eq!(cp.authorize(&req).await, Authz::Allow(Grant::default()));
        assert_eq!(cp.health_check("token", FROM).await, None);
        let res = cp.http_proxy(FROM, HttpProxyRequest::default()).await;
        assert!(!res.error.is_empty() && res.body.is_empty(), "{res:?}");
    }
}
