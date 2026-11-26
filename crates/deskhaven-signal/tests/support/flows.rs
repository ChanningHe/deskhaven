use super::{Conn, Rendezvous, STEP, TcpClient, TestServer, UdpClient};
use hbb_common::{
    rendezvous_proto::PunchHoleRequest,
    tokio::time::{Duration, Instant, sleep},
};

const RELAY_POLL: Duration = Duration::from_millis(50);

pub async fn relay_via(
    controller: &mut TcpClient,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
) -> String {
    controller.punch_hole_request(req.clone()).await;
    expect_msg!(target.recv().await, PunchHole).relay_server
}

/// A fresh connection per sample keeps the baseline tests transport-neutral.
pub async fn next_relay(
    server: &TestServer,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
) -> String {
    relay_via(&mut server.tcp().await, target, req).await
}

/// Relay lists apply asynchronously: the initial config at the first timer tick, handle updates
/// through the server loop.
pub async fn wait_for_relay(
    server: &TestServer,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
    accept: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + STEP;
    loop {
        let relay = next_relay(server, target, req).await;
        if accept(&relay) {
            return relay;
        }
        assert!(Instant::now() < deadline, "relay {relay:?} never accepted");
        sleep(RELAY_POLL).await;
    }
}
