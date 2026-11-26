use super::{PeerKeys, QUIET, STEP, io, msg, msg::rv};
use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{
        OnlineRequest, OnlineResponse, PunchHoleRequest, RegisterPeerResponse, RendezvousMessage,
        RequestRelay, TestNatRequest, TestNatResponse, register_pk_response,
    },
    tokio::time::Duration,
};

#[derive(Debug)]
pub enum Recv {
    Msg(Box<RendezvousMessage>),
    Closed,
    Quiet,
}

impl Recv {
    pub fn parse(bytes: &[u8]) -> Self {
        Self::Msg(Box::new(
            RendezvousMessage::parse_from_bytes(bytes).unwrap(),
        ))
    }
}

#[allow(async_fn_in_trait)] // test-only trait, never used as `dyn` or across threads
pub trait Conn {
    async fn send(&mut self, msg: &RendezvousMessage);

    async fn recv_within(&mut self, wait: Duration) -> Recv;

    async fn try_recv(&mut self, wait: Duration) -> Option<RendezvousMessage> {
        match self.recv_within(wait).await {
            Recv::Msg(msg) => Some(*msg),
            Recv::Closed | Recv::Quiet => None,
        }
    }

    async fn recv(&mut self) -> RendezvousMessage {
        match io("recv", self.recv_within(STEP)).await {
            Recv::Msg(msg) => *msg,
            other => panic!("no message within {STEP:?}: {other:?}"),
        }
    }

    async fn assert_silent(&mut self) {
        if let Recv::Msg(msg) = self.recv_within(QUIET).await {
            panic!("unexpected message: {msg:?}");
        }
    }

    async fn assert_closed(&mut self) {
        match io("assert_closed", self.recv_within(STEP)).await {
            Recv::Closed => {}
            other => panic!("expected the server to close the connection, got {other:?}"),
        }
    }
}

#[allow(async_fn_in_trait)]
pub trait Rendezvous: Conn {
    async fn punch_hole_request(&mut self, req: PunchHoleRequest) {
        self.send(&rv(|m| m.set_punch_hole_request(req))).await;
    }

    async fn request_relay(&mut self, req: RequestRelay) {
        self.send(&rv(|m| m.set_request_relay(req))).await;
    }

    async fn register_pk(&mut self, id: &str, keys: &PeerKeys) -> register_pk_response::Result {
        self.send(&msg::register_pk(id, keys)).await;
        let res = expect_msg!(self.recv().await, RegisterPkResponse);
        res.result.enum_value().unwrap()
    }

    /// RustDesk clients only send this over UDP.
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
