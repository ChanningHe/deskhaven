//! Policy hooks of the signal server; [`AllowAll`] reproduces the OSS behavior. Only
//! `pick_relay`, `peer_online` and `peer_offline` are called today: `register_pk`,
//! `authorize`, `http_proxy` and `health_check` are not wired yet, so no connection ACL
//! is enforced here.

use async_trait::async_trait;
use hbb_common::rendezvous_proto::{
    ConnType, ControlPermissions, HttpProxyRequest, HttpProxyResponse,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterVerdict {
    Ok,
    NotDeployed,
    UuidMismatch,
}

#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct RegisterInfo<'a> {
    pub id: &'a str,
    pub uuid: &'a [u8],
    pub pk: &'a [u8],
    pub ip: IpAddr,
    pub no_register_device: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnKind {
    Punch,
    Relay,
}

/// It names no initiator id: the protocol has none, identity can only come from `token`.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ConnectReq<'a> {
    pub kind: ConnKind,
    pub target: &'a str,
    pub token: &'a str,
    pub switch_code: &'a str,
    pub conn_type: ConnType,
    pub from: SocketAddr,
    pub secured: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grant {
    /// Injected into PunchHole / FetchLocalAddr / RequestRelay; `None` leaves the peer's own.
    pub permissions: Option<ControlPermissions>,
    pub audit_ref: String,
    /// `feedback` for the controller (non-zero asks it to keep a HealthCheck connection).
    pub feedback: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Authz {
    Allow(Grant),
    /// Refused; the text is shown to the controller as is.
    Deny(String),
}

/// Implementations must be cheap: they run on the request paths.
#[async_trait]
pub trait ControlPlane: Send + Sync + 'static {
    /// Called inline on the UDP loop: must answer from memory, never await I/O.
    fn register_pk(&self, r: &RegisterInfo<'_>) -> RegisterVerdict;

    /// Should answer from an in-memory snapshot, never a network round trip per request.
    async fn authorize(&self, r: &ConnectReq<'_>) -> Authz;

    async fn http_proxy(&self, from: SocketAddr, req: HttpProxyRequest) -> HttpProxyResponse;

    /// `Some(keep_alive_secs)` keeps the connection open, `None` refuses.
    async fn health_check(&self, token: &str, from: SocketAddr) -> Option<i32>;

    /// `relays` are the configured relays that resolved and passed the latest health check, in
    /// order; `None` lets clients fall back to their own relay setting. Must not block.
    fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String>;

    /// Called inline on the UDP loop and connection tasks, for every registered peer every few
    /// seconds: must return at once.
    fn peer_online(&self, _id: &str, _ip: IpAddr) {}

    /// Same constraints as [`peer_online`](Self::peer_online).
    fn peer_offline(&self, _id: &str) {}
}

#[derive(Debug, Default)]
pub struct AllowAll {
    /// Per instance, not process-wide.
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
