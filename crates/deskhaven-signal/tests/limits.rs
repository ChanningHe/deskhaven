//! Timeout tests run on tokio's paused clock and move it only with `advance` from a settled
//! server, waiting for real I/O with it held still: the paused clock otherwise leaps ahead
//! while loopback data is still in flight (see `support/clock.rs`).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::{PunchHoleRequest, punch_hole_response::Failure::ID_NOT_EXIST},
    tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        time::{Duration, Instant, timeout},
    },
};
use std::net::SocketAddr;
use support::{
    Conn, KxClient, Recv, Rendezvous, STEP, TestServer, answer, expect_msg, io, read_offer,
};

/// Mirror the server's constants of the same names.
const MAX_FRAME_LEN: usize = 64 * 1024;
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const MAIN_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
const NAT_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Covers the ticks the server's own timers add on the paused clock.
const SLACK: Duration = Duration::from_secs(1);
/// Largest length a hbb_common frame header can declare (4-byte header).
const MAX_DECLARABLE: usize = 0x3FFF_FFFF;

fn frame_header(len: usize) -> [u8; 4] {
    let head = u32::try_from(len << 2).unwrap() | 0x3;
    head.to_le_bytes()
}

/// Padded with a WebRTC offer, the largest field a client sends.
fn padded_punch_req(server: &TestServer, size: usize) -> PunchHoleRequest {
    PunchHoleRequest {
        webrtc_sdp_offer: "x".repeat(size),
        ..server.punch_req("no-such-peer")
    }
}

async fn closed_within(stream: &mut TcpStream, limit: Duration) -> Option<Duration> {
    let started = Instant::now();
    let mut buf = [0u8; 1024];
    loop {
        match timeout(
            limit.saturating_sub(started.elapsed()),
            stream.read(&mut buf),
        )
        .await
        {
            Err(_) => return None,
            Ok(Ok(0) | Err(_)) => return Some(started.elapsed()),
            Ok(Ok(_)) => {}
        }
    }
}

#[tokio::test]
async fn tcp_frame_declared_over_the_limit_closes_the_connection_at_once() {
    let server = TestServer::start().await;

    for addr in [server.main_addr(), server.nat_addr()] {
        for len in [MAX_FRAME_LEN + 1, MAX_DECLARABLE] {
            let mut stream = TcpStream::connect(addr).await.unwrap();

            stream.write_all(&frame_header(len)).await.unwrap();

            assert!(
                closed_within(&mut stream, STEP).await.is_some(),
                "{addr}: header declaring {len} bytes"
            );
        }
        assert!(
            server.tcp().await.test_nat().await.port > 0,
            "still serving"
        );
    }
    server.stop().await;
}

#[tokio::test]
async fn tcp_and_websocket_frames_up_to_the_limit_are_served_and_larger_ones_close() {
    let server = TestServer::start().await;
    let fits = padded_punch_req(&server, MAX_FRAME_LEN - 1024);
    let too_big = padded_punch_req(&server, MAX_FRAME_LEN + 1);
    let mut tcp_ok = server.tcp().await;
    let mut tcp_big = server.tcp().await;
    let mut ws_ok = server.ws().await;
    let mut ws_big = server.ws().await;

    tcp_ok.punch_hole_request(fits.clone()).await;
    tcp_big.punch_hole_request(too_big.clone()).await;
    ws_ok.punch_hole_request(fits).await;
    ws_big.punch_hole_request(too_big).await;

    expect_id_not_exist(&mut tcp_ok).await;
    expect_id_not_exist(&mut ws_ok).await;
    tcp_big.assert_closed().await;
    ws_big.assert_closed().await;
    server.stop().await;
}

async fn expect_id_not_exist(conn: &mut impl Conn) {
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

#[allow(async_fn_in_trait)] // test-only trait, never used as `dyn` or across threads
trait TimedClose {
    /// `false` once the server's close has reached us; a zero wait never moves the clock.
    async fn is_open(&mut self) -> bool;

    /// Waits for the server's close with the clock held still.
    async fn closed(&mut self);
}

impl TimedClose for TcpStream {
    async fn is_open(&mut self) -> bool {
        let mut buf = [0u8; 1024];
        loop {
            match self.try_read(&mut buf) {
                Ok(0) => return false,
                Ok(_) => {} // the main port's KeyExchange offer
                Err(e) => return e.kind() == std::io::ErrorKind::WouldBlock,
            }
        }
    }

    async fn closed(&mut self) {
        let mut buf = [0u8; 1024];
        let what = "the server closing a silent connection";
        io(what, async {
            while matches!(self.read(&mut buf).await, Ok(n) if n > 0) {}
        })
        .await;
    }
}

impl<C: Conn> TimedClose for C {
    async fn is_open(&mut self) -> bool {
        match self.recv_within(Duration::ZERO).await {
            Recv::Closed => false,
            Recv::Msg(msg) => panic!("unexpected message: {msg:?}"),
            Recv::Quiet => true,
        }
    }

    async fn closed(&mut self) {
        self.assert_closed().await;
    }
}

/// Still open `SLACK` before `limit`, closed `SLACK` after. The caller has settled the server,
/// so nothing is in flight when the paused clock moves.
async fn assert_closes_at(conn: &mut impl TimedClose, limit: Duration, what: &str) {
    tokio::time::advance(limit - SLACK).await;
    assert!(conn.is_open().await, "{what}: closed before {limit:?}");
    tokio::time::advance(SLACK * 2).await;
    conn.closed().await;
}

/// The answer proves the server read the request, and the clock has not moved since.
async fn assert_idle_timeout(
    conn: &mut (impl Rendezvous + TimedClose),
    server: &TestServer,
    name: &str,
) {
    conn.punch_hole_request(server.punch_req("idle-unknown"))
        .await;
    expect_id_not_exist(conn).await;
    assert_closes_at(conn, IDLE_TIMEOUT, name).await;
}

#[tokio::test(start_paused = true)]
async fn silent_connection_is_closed_after_the_first_frame_timeout() {
    let server = TestServer::start().await;
    let ports: [(SocketAddr, Duration, &str); 3] = [
        (server.main_addr(), MAIN_FIRST_FRAME_TIMEOUT, "main"),
        (server.nat_addr(), NAT_FIRST_FRAME_TIMEOUT, "nat"),
        (
            server.ws_addr(),
            FIRST_FRAME_TIMEOUT,
            "ws without handshake",
        ),
    ];

    for (addr, limit, name) in ports {
        let mut stream = TestServer::raw_connect(addr).await;
        server.settle().await;

        assert_closes_at(&mut stream, limit, name).await;
    }
    let mut ws = server.ws().await;
    assert_closes_at(&mut ws, FIRST_FRAME_TIMEOUT, "ws after handshake").await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn first_frame_within_the_first_frame_timeout_is_served() {
    let server = TestServer::start().await;
    let mut main = server.tcp().await;
    let mut nat = server.nat().await;
    server.settle().await;

    tokio::time::advance(NAT_FIRST_FRAME_TIMEOUT - SLACK * 2).await;
    assert!(nat.test_nat().await.port > 0, "nat");
    tokio::time::advance(MAIN_FIRST_FRAME_TIMEOUT - NAT_FIRST_FRAME_TIMEOUT).await;
    assert!(main.test_nat().await.port > 0, "main");
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn request_after_the_key_exchange_gets_the_idle_timeout() {
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    let offer = read_offer(&mut conn).await;
    tokio::time::advance(MAIN_FIRST_FRAME_TIMEOUT - SLACK * 2).await;
    answer(&mut conn, &server.public_key, KxClient::Current, &offer)
        .await
        .unwrap();
    // The reply has no answer to wait for; the witnesses connect after it was sent.
    server.settle().await;

    tokio::time::advance(IDLE_TIMEOUT - SLACK * 5).await;

    assert!(conn.test_nat().await.port > 0);
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn idle_connection_is_closed_after_the_idle_timeout() {
    let server = TestServer::start().await;

    assert_idle_timeout(&mut server.tcp().await, &server, "tcp").await;
    assert_idle_timeout(&mut server.ws().await, &server, "ws").await;
    server.stop().await;
}
