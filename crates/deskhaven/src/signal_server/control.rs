//! The signal crate must not depend on the core, and `PeerStore` never sees the keep-alives
//! that make a peer online; `ControlPlane::peer_online` / `peer_offline` are called exactly
//! where hbbs refreshes or expires a registration, inline and memory-only, as `Presence` is.

use std::net::{IpAddr, SocketAddr};

use async_trait::async_trait;
use deskhaven_core::devices::Presence;
use deskhaven_signal::{
    AllowAll, Authz, ConnectReq, ControlPlane, HttpProxyRequest, HttpProxyResponse, RegisterInfo,
    RegisterVerdict,
};

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

    #[tokio::test]
    async fn a_signal_registration_marks_the_device_online_in_presence() {
        let presence = Presence::new(Arc::new(SystemClock));
        let control: Arc<dyn ControlPlane> = Arc::new(CoreControl::new(presence.clone()));
        let (_tx, settings) = tokio::sync::watch::channel(Arc::new(with(&[])));
        let token = CancellationToken::new();
        let (running, port, _) =
            start_launcher_with(settings, memory_store(), control, FAST, &token).await;

        Target::register(port).await;

        wait_until(|| presence.is_online(TARGET)).await;
        assert_eq!(presence.get(TARGET).unwrap().ip, Some(TARGET_IP));
        token.cancel();
        running.task.await.unwrap();
    }
}
