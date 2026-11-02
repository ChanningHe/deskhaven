//! Transport-independent client surface.

use super::{PeerKeys, QUIET, STEP, msg, msg::rv};
use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{
        OnlineRequest, OnlineResponse, PunchHoleRequest, RegisterPeerResponse, RendezvousMessage,
        RequestRelay, TestNatRequest, TestNatResponse, register_pk_response,
    },
    tokio::time::Duration,
};

/// What one receive saw within its wait.
#[derive(Debug)]
pub enum Recv {
    Msg(Box<RendezvousMessage>),
    /// The server closed the connection (or reset it). Never happens on UDP.
    Closed,
    /// Nothing arrived and the connection is still open.
    Quiet,
}

impl Recv {
    /// A received payload; panics if it is not a `RendezvousMessage`.
    pub fn parse(bytes: &[u8]) -> Self {
        Self::Msg(Box::new(
            RendezvousMessage::parse_from_bytes(bytes).unwrap(),
        ))
    }
}

/// Sending and receiving `RendezvousMessage`s over one transport.
#[allow(async_fn_in_trait)] // test-only trait, never used as `dyn` or across threads
pub trait Conn {
    async fn send(&mut self, msg: &RendezvousMessage);

    async fn recv_within(&mut self, wait: Duration) -> Recv;

    /// The next message within `wait`; `None` on silence or when the server closed.
    async fn try_recv(&mut self, wait: Duration) -> Option<RendezvousMessage> {
        match self.recv_within(wait).await {
            Recv::Msg(msg) => Some(*msg),
            Recv::Closed | Recv::Quiet => None,
        }
    }

    /// The next message; panics if none arrives within [`STEP`].
    async fn recv(&mut self) -> RendezvousMessage {
        match self.recv_within(STEP).await {
            Recv::Msg(msg) => *msg,
            other => panic!("no message within {STEP:?}: {other:?}"),
        }
    }

    /// Panics if a message arrives within [`QUIET`]; a closed connection counts as silent.
    async fn assert_silent(&mut self) {
        if let Recv::Msg(msg) = self.recv_within(QUIET).await {
            panic!("unexpected message: {msg:?}");
        }
    }

    /// Panics unless the server closes the connection within [`STEP`] without sending anything.
    async fn assert_closed(&mut self) {
        match self.recv_within(STEP).await {
            Recv::Closed => {}
            other => panic!("expected the server to close the connection, got {other:?}"),
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

    /// A controlled peer in TCP / websocket mode registering its key (M1-08).
    async fn register_pk(&mut self, id: &str, keys: &PeerKeys) -> register_pk_response::Result {
        self.send(&msg::register_pk(id, keys)).await;
        let res = expect_msg!(self.recv().await, RegisterPkResponse);
        res.result.enum_value().unwrap()
    }

    /// RegisterPeer over a stream (M1-08; RustDesk clients only send it over UDP).
    async fn register_peer(&mut self, id: &str) -> RegisterPeerResponse {
        self.send(&msg::register_peer(id)).await;
        expect_msg!(self.recv().await, RegisterPeerResponse)
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
