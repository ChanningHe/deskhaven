//! Multi-step protocol flows shared by several test binaries.

use super::{Conn, Rendezvous, STEP, TcpClient, TestServer, UdpClient};
use hbb_common::{
    rendezvous_proto::PunchHoleRequest,
    tokio::time::{Duration, Instant, sleep},
};

const RELAY_POLL: Duration = Duration::from_millis(50);

/// The relay_server of the PunchHole forwarded to `target` for `req` sent on `controller`.
pub async fn relay_via(
    controller: &mut TcpClient,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
) -> String {
    controller.punch_hole_request(req.clone()).await;
    expect_msg!(target.recv().await, PunchHole).relay_server
}

/// [`relay_via`] a new controller connection. A connection keeps the relay list that was
/// current when it was accepted (rs:1174-1182 clones the server state into each connection
/// task), so only new connections see a new list.
pub async fn next_relay(
    server: &TestServer,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
) -> String {
    relay_via(&mut server.tcp().await, target, req).await
}

/// Repeats [`next_relay`] until the relay satisfies `accept`: relay lists apply asynchronously
/// (initial config at the first timer tick, handle updates through the server loop).
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
