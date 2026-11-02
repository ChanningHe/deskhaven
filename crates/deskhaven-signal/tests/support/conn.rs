//! Transport-independent client surface.

use super::{QUIET, STEP, msg::rv};
use hbb_common::{
    rendezvous_proto::{
        OnlineRequest, OnlineResponse, PunchHoleRequest, RendezvousMessage, RequestRelay,
        TestNatRequest, TestNatResponse,
    },
    tokio::time::Duration,
};

/// Sending and receiving `RendezvousMessage`s over one transport.
#[allow(async_fn_in_trait)] // test-only trait, never used as `dyn` or across threads
pub trait Conn {
    async fn send(&mut self, msg: &RendezvousMessage);

    /// The next message within `wait`; `None` on timeout or when the server closed.
    async fn try_recv(&mut self, wait: Duration) -> Option<RendezvousMessage>;

    /// The next message; panics if none arrives within [`STEP`].
    async fn recv(&mut self) -> RendezvousMessage {
        match self.try_recv(STEP).await {
            Some(msg) => msg,
            None => panic!("no message within {STEP:?}"),
        }
    }

    /// Panics if a message arrives within [`QUIET`].
    async fn assert_silent(&mut self) {
        if let Some(msg) = self.try_recv(QUIET).await {
            panic!("unexpected message: {msg:?}");
        }
    }
}

/// Typed requests a client sends over a stream transport (TCP main / NAT port, websocket).
#[allow(async_fn_in_trait)]
pub trait Rendezvous: Conn {
    /// Controller asks hbbs to reach `req.id`; answers arrive later through [`Conn::recv`].
    async fn punch_hole_request(&mut self, req: PunchHoleRequest) {
        self.send(&rv(|m| m.set_punch_hole_request(req))).await;
    }

    /// Controller asks hbbs to pass a relay request on to `req.id` (no direct answer).
    async fn request_relay(&mut self, req: RequestRelay) {
        self.send(&rv(|m| m.set_request_relay(req))).await;
    }

    async fn test_nat(&mut self) -> TestNatResponse {
        self.send(&rv(|m| m.set_test_nat_request(TestNatRequest::default())))
            .await;
        expect_msg!(self.recv().await, TestNatResponse)
    }

    /// Only the NAT test port (main port - 1) answers this.
    async fn online_request(&mut self, ids: &[&str]) -> OnlineResponse {
        let req = OnlineRequest {
            id: "online-asker".to_owned(),
            peers: ids.iter().map(|id| (*id).to_owned()).collect(),
            ..Default::default()
        };
        self.send(&rv(|m| m.set_online_request(req))).await;
        expect_msg!(self.recv().await, OnlineResponse)
    }
}
