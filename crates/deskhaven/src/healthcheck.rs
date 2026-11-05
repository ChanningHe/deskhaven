//! `deskhaven healthcheck`: asks the local `/readyz` whether the server is
//! ready. For container health checks in images without a shell or curl
//! (distroless); the exit code is the result.

use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use deskhaven_config::bootstrap::{Listen, LoadOptions};

use crate::config_cmd;

/// The whole probe (connect, request, status line) must finish within this.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// The status line is all that is read; a longer first line is malformed.
const MAX_STATUS_LINE: usize = 256;
const READY: u16 = 200;

#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("cannot connect to {addr}: {source}")]
    Unreachable { addr: SocketAddr, source: io::Error },
    #[error("request to {addr} failed: {source}")]
    Request { addr: SocketAddr, source: io::Error },
    #[error("no answer from {addr} within {timeout:?}")]
    TimedOut { addr: SocketAddr, timeout: Duration },
    #[error("malformed response from {addr}")]
    Malformed { addr: SocketAddr },
    #[error("not ready: {addr}/readyz answered {status}")]
    NotReady { addr: SocketAddr, status: u16 },
}

/// Loads the configuration exactly as `serve` does and probes the client API
/// listener (the health routes are on every listener).
pub fn run(options: &LoadOptions) -> anyhow::Result<()> {
    let loaded = config_cmd::load_reporting(options)?;
    probe(probe_addr(&loaded.config.listen), TIMEOUT)?;
    Ok(())
}

/// Where a local client reaches the client API listener: a wildcard bind
/// address becomes the loopback address of the same family.
pub fn probe_addr(listen: &Listen) -> SocketAddr {
    let addr = listen.client_api;
    let ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, addr.port())
}

/// `GET /readyz` on `addr`; `Ok` only for a 200. Everything, connect
/// included, shares one `timeout` budget.
pub fn probe(addr: SocketAddr, timeout: Duration) -> Result<(), HealthError> {
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|source| {
        if is_timeout(&source) {
            HealthError::TimedOut { addr, timeout }
        } else {
            HealthError::Unreachable { addr, source }
        }
    })?;
    let request =
        format!("GET /readyz HTTP/1.1\r\nHost: {addr}\r\nAccept: */*\r\nConnection: close\r\n\r\n");
    let io_error = |source: io::Error| {
        if is_timeout(&source) {
            HealthError::TimedOut { addr, timeout }
        } else {
            HealthError::Request { addr, source }
        }
    };
    stream
        .set_write_timeout(Some(remaining(deadline, addr, timeout)?))
        .and_then(|()| stream.write_all(request.as_bytes()))
        .map_err(io_error)?;
    let line = read_status_line(&mut stream, deadline, addr, timeout)?;
    match parse_status(&line) {
        Some(READY) => Ok(()),
        Some(status) => Err(HealthError::NotReady { addr, status }),
        None => Err(HealthError::Malformed { addr }),
    }
}

/// Bytes up to the first CRLF (or EOF, or [`MAX_STATUS_LINE`]).
fn read_status_line(
    stream: &mut TcpStream,
    deadline: Instant,
    addr: SocketAddr,
    timeout: Duration,
) -> Result<Vec<u8>, HealthError> {
    let mut line = Vec::new();
    let mut buf = [0u8; MAX_STATUS_LINE];
    while !line.windows(2).any(|w| w == b"\r\n") && line.len() < MAX_STATUS_LINE {
        stream
            .set_read_timeout(Some(remaining(deadline, addr, timeout)?))
            .map_err(|source| HealthError::Request { addr, source })?;
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(source) if is_timeout(&source) => {
                return Err(HealthError::TimedOut { addr, timeout });
            }
            Err(source) => return Err(HealthError::Request { addr, source }),
        };
        line.extend_from_slice(&buf[..n]);
    }
    Ok(line)
}

/// The status code of `HTTP/1.x <code> ...`.
fn parse_status(response: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(response.get(..response.len().min(MAX_STATUS_LINE))?).ok()?;
    let line = text.split("\r\n").next()?;
    let mut parts = line.splitn(3, ' ');
    let version = parts.next()?;
    let code = parts.next()?;
    let is_http1 = version == "HTTP/1.1" || version == "HTTP/1.0";
    let is_code = code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit());
    (is_http1 && is_code).then(|| code.parse().ok()).flatten()
}

fn remaining(
    deadline: Instant,
    addr: SocketAddr,
    timeout: Duration,
) -> Result<Duration, HealthError> {
    // A zero timeout means "block forever" to the socket API.
    Some(deadline.saturating_duration_since(Instant::now()))
        .filter(|left| !left.is_zero())
        .ok_or(HealthError::TimedOut { addr, timeout })
}

fn is_timeout(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::mpsc::{self, Receiver};

    use super::*;

    const SHORT: Duration = Duration::from_millis(300);
    /// Bounds every wait of the fake server, so a probe that never connects
    /// fails the test instead of hanging it.
    const SERVER_LIMIT: Duration = Duration::from_secs(3);
    const ACCEPT_POLL: Duration = Duration::from_millis(10);

    fn listen(client_api: &str) -> Listen {
        Listen {
            client_api: client_api.parse().unwrap(),
            ..Listen::default()
        }
    }

    /// A one-shot server: reads the request head, answers `response`, and
    /// sends back what it received (an error if nobody connected in time).
    fn serve_once(response: &'static str) -> (SocketAddr, Receiver<Result<String, String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(answer_once(&listener, response));
        });
        (addr, rx)
    }

    fn answer_once(listener: &TcpListener, response: &str) -> Result<String, String> {
        let deadline = Instant::now() + SERVER_LIMIT;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            }
            if Instant::now() > deadline {
                return Err("nobody connected".to_owned());
            }
            std::thread::sleep(ACCEPT_POLL);
        };
        let setup = stream
            .set_nonblocking(false)
            .and_then(|()| stream.set_read_timeout(Some(SERVER_LIMIT)))
            .and_then(|()| stream.set_write_timeout(Some(SERVER_LIMIT)));
        setup.map_err(|e| e.to_string())?;
        let mut request = Vec::new();
        let mut buf = [0u8; 512];
        while !request.ends_with(b"\r\n\r\n") {
            match stream.read(&mut buf).map_err(|e| e.to_string())? {
                0 => break,
                n => request.extend_from_slice(&buf[..n]),
            }
        }
        stream
            .write_all(response.as_bytes())
            .map_err(|e| e.to_string())?;
        String::from_utf8(request).map_err(|e| e.to_string())
    }

    /// What the fake server received; bounded wait.
    fn received(server: &Receiver<Result<String, String>>) -> String {
        server
            .recv_timeout(SERVER_LIMIT * 2)
            .expect("fake server did not finish")
            .expect("fake server failed")
    }

    #[test]
    fn wildcard_binds_are_probed_on_loopback() {
        assert_eq!(
            probe_addr(&listen("0.0.0.0:21114")),
            "127.0.0.1:21114".parse().unwrap()
        );
        assert_eq!(
            probe_addr(&listen("[::]:21114")),
            "[::1]:21114".parse().unwrap()
        );
        assert_eq!(
            probe_addr(&listen("10.1.2.3:8080")),
            "10.1.2.3:8080".parse().unwrap()
        );
    }

    #[test]
    fn ready_when_readyz_answers_200() {
        let (addr, server) = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");

        let result = probe(addr, SHORT);

        assert!(result.is_ok(), "{result:?}");
        let request = received(&server);
        assert!(request.starts_with("GET /readyz HTTP/1.1\r\n"), "{request}");
        assert!(request.contains(&format!("Host: {addr}\r\n")), "{request}");
    }

    #[test]
    fn not_ready_when_readyz_answers_503() {
        let (addr, server) = serve_once("HTTP/1.1 503 Service Unavailable\r\n\r\n");

        let result = probe(addr, SHORT);

        assert!(
            matches!(result, Err(HealthError::NotReady { status: 503, .. })),
            "{result:?}"
        );
        received(&server);
    }

    #[test]
    fn unreachable_when_nothing_listens() {
        let addr = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();

        let result = probe(addr, SHORT);

        assert!(
            matches!(result, Err(HealthError::Unreachable { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn malformed_when_the_answer_is_not_http() {
        let (addr, server) = serve_once("SSH-2.0-OpenSSH\r\n");

        let result = probe(addr, SHORT);

        assert!(
            matches!(result, Err(HealthError::Malformed { .. })),
            "{result:?}"
        );
        received(&server);
    }

    #[test]
    fn a_silent_server_times_out_quickly() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let started = Instant::now();

        let result = probe(addr, SHORT);

        assert!(
            matches!(result, Err(HealthError::TimedOut { .. })),
            "{result:?}"
        );
        assert!(started.elapsed() < SHORT * 3, "{:?}", started.elapsed());
        drop(listener);
    }
}
