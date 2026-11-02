//! The control plane `serve` gives the signal server: upstream's policy ([`AllowAll`]) until
//! M5, plus the liveness notifications, which feed the device presence table of the core
//! (`deskhaven_core::devices::Presence`, M2-12).
//!
//! Why this path: the signal crate must not depend on the core (02 §2.1), and its only
//! injected interfaces are `ControlPlane` and `PeerStore`. `PeerStore` only sees changed
//! registrations, written behind and coalesced, never the keep-alives that make a peer
//! online; `ControlPlane::peer_online` / `peer_offline` are called exactly where hbbs
//! refreshes or expires a registration, inline and memory-only, which is what `Presence`
//! is (a short mutex section, bounded).

use std::net::{IpAddr, SocketAddr};

use async_trait::async_trait;
use deskhaven_core::devices::Presence;
use deskhaven_signal::{
    AllowAll, Authz, ConnectReq, ControlPlane, HttpProxyRequest, HttpProxyResponse, RegisterInfo,
    RegisterVerdict,
};

/// [`AllowAll`] decisions; liveness goes to [`Presence`].
pub struct CoreControl {
    policy: AllowAll,
    presence: Presence,
}

impl CoreControl {
    pub fn new(presence: Presence) -> Self {
        Self {
            policy: AllowAll::new(),
            presence,
        }
    }
}

#[async_trait]
impl ControlPlane for CoreControl {
    fn register_pk(&self, r: &RegisterInfo<'_>) -> RegisterVerdict {
        self.policy.register_pk(r)
    }

    async fn authorize(&self, r: &ConnectReq<'_>) -> Authz {
        self.policy.authorize(r).await
    }

    async fn http_proxy(&self, from: SocketAddr, req: HttpProxyRequest) -> HttpProxyResponse {
        self.policy.http_proxy(from, req).await
    }

    async fn health_check(&self, token: &str, from: SocketAddr) -> Option<i32> {
        self.policy.health_check(token, from).await
    }

    fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String> {
        self.policy.pick_relay(a, b, relays)
    }

    fn peer_online(&self, id: &str, ip: IpAddr) {
        self.presence.signal_online(id, ip);
    }

    fn peer_offline(&self, id: &str) {
        self.presence.signal_offline(id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use deskhaven_core::clock::SystemClock;
    use deskhaven_signal::ControlPlane;
    use tokio_util::sync::CancellationToken;

    use super::super::tests::{
        FAST, TARGET, TARGET_IP, Target, memory_store, start_launcher_with, wait_until, with,
    };
    use super::*;

    /// A real RegisterPk over UDP reaches the presence table through the signal server's
    /// liveness notification: the device is online, seen from its source address.
    #[tokio::test]
    async fn a_signal_registration_marks_the_device_online_in_presence() {
        // Arrange
        let presence = Presence::new(Arc::new(SystemClock));
        let control: Arc<dyn ControlPlane> = Arc::new(CoreControl::new(presence.clone()));
        let (_tx, settings) = tokio::sync::watch::channel(Arc::new(with(&[])));
        let token = CancellationToken::new();
        let (running, port, _) =
            start_launcher_with(settings, memory_store(), control, FAST, &token).await;

        // Act
        Target::register(port).await;

        // Assert
        wait_until(|| presence.is_online(TARGET)).await;
        assert_eq!(presence.get(TARGET).unwrap().ip, Some(TARGET_IP));
        token.cancel();
        running.task.await.unwrap();
    }

    /// The decisions stay upstream's (M5 replaces them).
    #[tokio::test]
    async fn decisions_are_allow_all() {
        let control = CoreControl::new(Presence::new(Arc::new(SystemClock)));
        let relays = ["r1:21117".to_owned()];
        let ip = IpAddr::from([127, 0, 0, 1]);

        assert_eq!(
            control.pick_relay(ip, ip, &relays).as_deref(),
            Some("r1:21117")
        );
        assert_eq!(control.health_check("t", (ip, 1).into()).await, None);
    }
}
