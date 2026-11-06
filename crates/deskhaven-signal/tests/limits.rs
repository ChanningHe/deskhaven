//! M1-08: frame size and time limits of the stream ports (TCP main / NAT test, websocket).
//!
//! Upstream decoded frames without a size limit (hbb_common `BytesCodec::new()` accepts the
//! 1 GiB its header can declare; tungstenite's defaults allow 64 MiB messages), gave every frame
//! 30 s and the websocket handshake no time limit at all. Now a frame over [`MAX_FRAME_LEN`]
//! closes the connection as soon as its header is read, the first frame (websocket: handshake
//! included) must arrive within [`FIRST_FRAME_TIMEOUT`] ([`NAT_FIRST_FRAME_TIMEOUT`] on the NAT
//! test port) and later ones within
//! [`IDLE_TIMEOUT`] of the previous one. The timeout tests run on tokio's paused clock so they take
//! no real time. They move it only with `advance`, from a state where the server has caught up
//! ([`TestServer::settle`]), and wait for real I/O events with it held still ([`io`]): the
//! paused clock otherwise leaps ahead whenever the runtime finds nothing ready, which includes
//! loopback data still in flight on a loaded machine (see `support/clock.rs`).

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

/// The server's frame limit (`rendezvous_server.rs` `MAX_FRAME_LEN`).
const MAX_FRAME_LEN: usize = 64 * 1024;
/// The server's limit for the first frame on the websocket port, handshake included
/// (`FIRST_FRAME_TIMEOUT`, upstream's 30 s per frame).
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
/// The same on the main port, from the KeyExchange offer (`MAIN_FIRST_FRAME_TIMEOUT`,
/// M4-05 review H-A).
const MAIN_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(15);
/// The same on the NAT test port (`NAT_FIRST_FRAME_TIMEOUT`, M1-07): clients send their
/// TestNatRequest / OnlineRequest right after connecting.
const NAT_FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// The server's limit between frames (`IDLE_TIMEOUT`).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Slack around a timeout on the paused clock (timers fire exactly; this covers the ticks
/// the server's own timers add).
const SLACK: Duration = Duration::from_secs(1);
/// Largest length a hbb_common frame header can declare (4-byte header).
const MAX_DECLARABLE: usize = 0x3FFF_FFFF;

/// A 4-byte `BytesCodec` header declaring `len` payload bytes.
fn frame_header(len: usize) -> [u8; 4] {
    let head = u32::try_from(len << 2).unwrap() | 0x3;
    head.to_le_bytes()
}

/// A PunchHoleRequest for an unknown id whose encoding is about `size` bytes (padded with a
/// WebRTC offer, the largest field a client sends).
fn padded_punch_req(server: &TestServer, size: usize) -> PunchHoleRequest {
    PunchHoleRequest {
        webrtc_sdp_offer: "x".repeat(size),
        ..server.punch_req("no-such-peer")
    }
}

/// How long until the server closes `stream`, reading (and discarding) what it sends; `None`
/// if it is still open after `limit`.
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

/// A frame header declaring more than [`MAX_FRAME_LEN`] closes the connection at once, before
/// any payload is sent (so nothing is buffered for it); upstream waited for the payload.
#[tokio::test]
async fn tcp_frame_declared_over_the_limit_closes_the_connection_at_once() {
    // Arrange
    let server = TestServer::start().await;

    for addr in [server.main_addr(), server.nat_addr()] {
        for len in [MAX_FRAME_LEN + 1, MAX_DECLARABLE] {
            let mut stream = TcpStream::connect(addr).await.unwrap();

            // Act
            stream.write_all(&frame_header(len)).await.unwrap();

            // Assert
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

/// Frames up to the limit are served (a large WebRTC offer fits); one over it closes the
/// connection without an answer. Upstream answered both.
#[tokio::test]
async fn tcp_and_websocket_frames_up_to_the_limit_are_served_and_larger_ones_close() {
    // Arrange
    let server = TestServer::start().await;
    let fits = padded_punch_req(&server, MAX_FRAME_LEN - 1024);
    let too_big = padded_punch_req(&server, MAX_FRAME_LEN + 1);
    let mut tcp_ok = server.tcp().await;
    let mut tcp_big = server.tcp().await;
    let mut ws_ok = server.ws().await;
    let mut ws_big = server.ws().await;

    // Act
    tcp_ok.punch_hole_request(fits.clone()).await;
    tcp_big.punch_hole_request(too_big.clone()).await;
    ws_ok.punch_hole_request(fits).await;
    ws_big.punch_hole_request(too_big).await;

    // Assert
    expect_id_not_exist(&mut tcp_ok).await;
    expect_id_not_exist(&mut ws_ok).await;
    tcp_big.assert_closed().await;
    ws_big.assert_closed().await;
    server.stop().await;
}

/// The answer to a punch hole for an unknown id (the connection stays open).
async fn expect_id_not_exist(conn: &mut impl Conn) {
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

/// A connection the server closes on a timer, seen from a client that only listens.
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

/// Checks that the server closes `conn` `limit` after it started its timer for it: still open
/// `SLACK` before, closed `SLACK` after (the slack covers the ticks the server's own timers add).
/// The caller has settled the server, so nothing is in flight when the paused clock moves;
/// the close itself is awaited as an event (see `support/clock.rs`).
async fn assert_closes_at(conn: &mut impl TimedClose, limit: Duration, what: &str) {
    tokio::time::advance(limit - SLACK).await;
    assert!(conn.is_open().await, "{what}: closed before {limit:?}");
    tokio::time::advance(SLACK * 2).await;
    conn.closed().await;
}

/// Punches an unknown id over `conn`, then checks it closes IDLE_TIMEOUT after the server read
/// the request (the answer proves it did, and the clock has not moved since).
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

/// A connection that sends nothing (websocket: not even the handshake) is closed after its
/// port's first-frame limit: FIRST_FRAME_TIMEOUT on the main and websocket ports (upstream's
/// 30 s; the main port's KeyExchange offer is read and discarded), NAT_FIRST_FRAME_TIMEOUT on
/// the NAT test port. Upstream never closed a websocket connection that did not complete its
/// handshake.
#[tokio::test(start_paused = true)]
async fn silent_connection_is_closed_after_the_first_frame_timeout() {
    // Arrange
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
        // Act
        let mut stream = TestServer::raw_connect(addr).await;
        server.settle().await;

        // Assert
        assert_closes_at(&mut stream, limit, name).await;
    }
    let mut ws = server.ws().await;
    assert_closes_at(&mut ws, FIRST_FRAME_TIMEOUT, "ws after handshake").await;
    server.stop().await;
}

/// A first frame shortly before its port's limit is still served: on the main port after
/// 13 s (an unlogged controller's first frame can take half its whole connect time, DNS and
/// failed addresses included; M1-07 review), on the NAT test port after 8 s. Paused clock.
#[tokio::test(start_paused = true)]
async fn first_frame_within_the_first_frame_timeout_is_served() {
    // Arrange
    let server = TestServer::start().await;
    let mut main = server.tcp().await;
    let mut nat = server.nat().await;
    server.settle().await;

    // Act + Assert
    tokio::time::advance(NAT_FIRST_FRAME_TIMEOUT - SLACK * 2).await;
    assert!(nat.test_nat().await.port > 0, "nat");
    tokio::time::advance(MAIN_FIRST_FRAME_TIMEOUT - NAT_FIRST_FRAME_TIMEOUT).await;
    assert!(main.test_nat().await.port > 0, "main");
    server.stop().await;
}

/// M1-07: the KeyExchange reply is a connection's first frame, so a client that answers the
/// offer late still has IDLE_TIMEOUT from then on for its request, well past
/// FIRST_FRAME_TIMEOUT from the accept. Paused clock.
#[tokio::test(start_paused = true)]
async fn request_after_the_key_exchange_gets_the_idle_timeout() {
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    let offer = read_offer(&mut conn).await;
    tokio::time::advance(MAIN_FIRST_FRAME_TIMEOUT - SLACK * 2).await;
    answer(&mut conn, &server.public_key, KxClient::Current, &offer)
        .await
        .unwrap();
    // The reply has no answer to wait for; the witnesses connect after it was sent.
    server.settle().await;

    // Act
    tokio::time::advance(IDLE_TIMEOUT - SLACK * 5).await;

    // Assert
    assert!(conn.test_nat().await.port > 0);
    server.stop().await;
}

/// After a first frame that keeps the connection open, it stays open until it has been silent
/// for IDLE_TIMEOUT (upstream's 30 s per frame), on TCP and websocket.
#[tokio::test(start_paused = true)]
async fn idle_connection_is_closed_after_the_idle_timeout() {
    // Arrange
    let server = TestServer::start().await;

    // Act + Assert
    assert_idle_timeout(&mut server.tcp().await, &server, "tcp").await;
    assert_idle_timeout(&mut server.ws().await, &server, "ws").await;
    server.stop().await;
}
