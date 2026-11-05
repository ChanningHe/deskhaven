use std::net::{Shutdown, SocketAddrV6, TcpListener};
use std::sync::mpsc::{self, Receiver};

use super::*;

/// Budget for probes that should succeed or fail fast: generous, the fake
/// server side is bounded on its own.
const GENEROUS: Duration = Duration::from_secs(5);
/// Budget for the timeout tests.
const SHORT: Duration = Duration::from_millis(600);
/// Scheduling slack below a deadline (timers may fire a little early).
const SLACK: Duration = Duration::from_millis(50);
/// Upper bound past a deadline on a loaded test machine.
const LATE: Duration = Duration::from_secs(2);
/// Bounds every wait of the fake server, so a probe that never connects
/// fails the test instead of hanging it.
const SERVER_LIMIT: Duration = Duration::from_secs(3);
const ACCEPT_POLL: Duration = Duration::from_millis(10);

type Script = Box<dyn FnOnce(&mut TcpStream) -> io::Result<()> + Send>;

fn listen(client_api: SocketAddr) -> Listen {
    Listen {
        client_api,
        ..Listen::default()
    }
}

/// A one-shot server: reads the request head, runs `script` on the
/// connection, then drains it until the client closes. Sends back the
/// request it received (an error if nobody connected in time).
fn serve(script: Script) -> (SocketAddr, Receiver<Result<String, String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(answer(&listener, script).map_err(|e| e.to_string()));
    });
    (addr, rx)
}

/// Answers `response`, then half-closes: the probe sees EOF after it.
fn serve_once(response: &'static str) -> (SocketAddr, Receiver<Result<String, String>>) {
    serve(Box::new(move |stream| {
        stream.write_all(response.as_bytes())?;
        stream.shutdown(Shutdown::Write)
    }))
}

fn accept_within(listener: &TcpListener, limit: Duration) -> io::Result<TcpStream> {
    let deadline = Instant::now() + limit;
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                std::thread::sleep(ACCEPT_POLL);
            }
            Err(e) => return Err(e),
        }
    }
}

fn answer(listener: &TcpListener, script: Script) -> io::Result<String> {
    let mut stream = accept_within(listener, SERVER_LIMIT)?;
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(SERVER_LIMIT))?;
    stream.set_write_timeout(Some(SERVER_LIMIT))?;
    let mut request = Vec::new();
    let mut buf = [0u8; 512];
    while !request.ends_with(b"\r\n\r\n") {
        match stream.read(&mut buf)? {
            0 => break,
            n => request.extend_from_slice(&buf[..n]),
        }
    }
    script(&mut stream)?;
    // Until the probe hangs up (or the read times out): a closed server
    // socket must not be what ends the probe.
    while matches!(stream.read(&mut buf), Ok(n) if n > 0) {}
    String::from_utf8(request).map_err(io::Error::other)
}

/// What the fake server received; bounded wait.
fn received(server: &Receiver<Result<String, String>>) -> String {
    server
        .recv_timeout(SERVER_LIMIT * 3)
        .expect("fake server did not finish")
        .expect("fake server failed")
}

#[test]
fn wildcard_binds_are_probed_on_loopback() {
    let probe = |s: &str| probe_addr(&listen(s.parse().unwrap()));

    assert_eq!(probe("0.0.0.0:21114"), "127.0.0.1:21114".parse().unwrap());
    assert_eq!(probe("[::]:21114"), "[::1]:21114".parse().unwrap());
    assert_eq!(probe("10.1.2.3:8080"), "10.1.2.3:8080".parse().unwrap());
}

#[test]
fn an_ipv6_wildcard_keeps_scope_and_flow_info() {
    let bind = SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 21114, 7, 3);

    let addr = probe_addr(&listen(bind.into()));

    let SocketAddr::V6(addr) = addr else {
        panic!("{addr} is not IPv6")
    };
    assert_eq!(*addr.ip(), Ipv6Addr::LOCALHOST);
    assert_eq!(
        (addr.port(), addr.flowinfo(), addr.scope_id()),
        (21114, 7, 3)
    );
}

#[test]
fn status_lines_are_parsed_on_bytes() {
    assert_eq!(parse_status(b"HTTP/1.1 200 OK\r\n"), Some(200));
    assert_eq!(parse_status(b"HTTP/1.0 503\r\nx: y\r\n"), Some(503));
    // The reason phrase is not decoded.
    assert_eq!(parse_status(b"HTTP/1.1 200 \xff\xfe\r\n"), Some(200));
    for bad in [
        &b"HTTP/2 200 OK\r\n"[..],
        b"HTTP/1.1 20 OK\r\n",
        b"HTTP/1.1 2000 OK\r\n",
        b"HTTP/1.1 2x0 OK\r\n",
        b"HTTP/1.1  200 OK\r\n",
        b"http/1.1 200 OK\r\n",
        b"HTTP/1.1 200 OK",
        b"",
    ] {
        assert_eq!(
            parse_status(bad),
            None,
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn a_crlf_past_the_status_line_limit_does_not_count() {
    let mut long = b"HTTP/1.1 200 ".to_vec();
    long.resize(MAX_STATUS_LINE, b'x');
    long.extend_from_slice(b"\r\n");

    assert_eq!(parse_status(&long), None);
}

#[test]
fn ready_when_readyz_answers_200() {
    let (addr, server) = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");

    let result = probe(addr, GENEROUS);

    assert!(result.is_ok(), "{result:?}");
    let request = received(&server);
    assert!(request.starts_with("GET /readyz HTTP/1.1\r\n"), "{request}");
    assert!(request.contains(&format!("Host: {addr}\r\n")), "{request}");
}

#[test]
fn a_status_line_split_across_writes_is_reassembled() {
    let (addr, server) = serve(Box::new(|stream| {
        stream.write_all(b"HTTP/1.")?;
        stream.flush()?;
        std::thread::sleep(Duration::from_millis(100));
        stream.write_all(b"1 200 OK\r\n\r\n")
    }));

    let result = probe(addr, GENEROUS);

    assert!(result.is_ok(), "{result:?}");
    received(&server);
}

#[test]
fn not_ready_for_any_other_status() {
    for (response, expected) in [
        ("HTTP/1.1 503 Service Unavailable\r\n\r\n", 503),
        ("HTTP/1.1 500 Internal Server Error\r\n\r\n", 500),
    ] {
        let (addr, server) = serve_once(response);

        let result = probe(addr, GENEROUS);

        assert!(
            matches!(result, Err(HealthError::NotReady { status, .. }) if status == expected),
            "{result:?}"
        );
        received(&server);
    }
}

#[test]
fn unreachable_when_nothing_listens() {
    let addr = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let result = probe(addr, GENEROUS);

    assert!(
        matches!(result, Err(HealthError::Unreachable { .. })),
        "{result:?}"
    );
}

#[test]
fn malformed_answers() {
    let oversized: &'static str = "A".repeat(MAX_STATUS_LINE * 2).leak();
    for response in ["SSH-2.0-OpenSSH\r\n", "", oversized] {
        let (addr, server) = serve_once(response);

        let result = probe(addr, GENEROUS);

        assert!(
            matches!(result, Err(HealthError::Malformed { .. })),
            "{:.40}: {result:?}",
            response
        );
        received(&server);
    }
}

fn assert_timed_out_near(result: Result<(), HealthError>, took: Duration, budget: Duration) {
    assert!(
        matches!(result, Err(HealthError::TimedOut { .. })),
        "{result:?}"
    );
    assert!(took + SLACK >= budget, "gave up early: {took:?}");
    assert!(took < budget + LATE, "gave up late: {took:?}");
}

#[test]
fn a_silent_server_times_out_on_the_budget() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let started = Instant::now();

    let result = probe(addr, SHORT);

    assert_timed_out_near(result, started.elapsed(), SHORT);
    drop(listener);
}

/// One byte every third of the budget: every single read succeeds well
/// within the budget, so only a total deadline ends the probe in time.
#[test]
fn a_trickling_server_hits_the_total_deadline() {
    let (addr, server) = serve(Box::new(|stream| {
        let started = Instant::now();
        while started.elapsed() < SERVER_LIMIT {
            if stream.write_all(b"H").is_err() {
                break;
            }
            std::thread::sleep(SHORT / 3);
        }
        Ok(())
    }));
    let started = Instant::now();

    let result = probe(addr, SHORT);

    assert_timed_out_near(result, started.elapsed(), SHORT);
    received(&server);
}
