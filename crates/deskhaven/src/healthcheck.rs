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
/// Only the status line is read; one without a CRLF within this is malformed.
const MAX_STATUS_LINE: usize = 256;
const READY: u16 = 200;
const CRLF: &[u8] = b"\r\n";
const STATUS_DIGITS: usize = 3;

/// Causes are in the `source` chain, not repeated in the messages.
#[derive(Debug, thiserror::Error)]
pub enum HealthError {
    #[error("cannot connect to {addr}")]
    Unreachable { addr: SocketAddr, source: io::Error },
    #[error("request to {addr} failed")]
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
/// address becomes the loopback address of the same family (IPv6 scope and
/// flow info kept).
pub fn probe_addr(listen: &Listen) -> SocketAddr {
    let mut addr = listen.client_api;
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => addr.set_ip(Ipv4Addr::LOCALHOST.into()),
        IpAddr::V6(ip) if ip.is_unspecified() => addr.set_ip(Ipv6Addr::LOCALHOST.into()),
        _ => {}
    }
    addr
}

/// `GET /readyz` on `addr`; `Ok` only for a 200. Everything, connect
/// included, shares one `timeout` budget.
pub fn probe(addr: SocketAddr, timeout: Duration) -> Result<(), HealthError> {
    let probe = Probe {
        addr,
        timeout,
        deadline: Instant::now() + timeout,
    };
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| probe.fail(e, true))?;
    probe.send(&mut stream)?;
    let response = probe.read_status_line(&mut stream)?;
    match parse_status(&response) {
        Some(READY) => Ok(()),
        Some(status) => Err(HealthError::NotReady { addr, status }),
        None => Err(HealthError::Malformed { addr }),
    }
}

#[derive(Clone, Copy)]
struct Probe {
    addr: SocketAddr,
    timeout: Duration,
    deadline: Instant,
}

impl Probe {
    fn send(&self, stream: &mut TcpStream) -> Result<(), HealthError> {
        let request = format!(
            "GET /readyz HTTP/1.1\r\nHost: {}\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            self.addr
        );
        stream
            .set_write_timeout(Some(self.remaining()?))
            .map_err(|e| self.fail(e, false))?;
        // `write_all` retries on `Interrupted` itself.
        stream
            .write_all(request.as_bytes())
            .map_err(|e| self.fail(e, false))
    }

    /// Up to the first CRLF, EOF or [`MAX_STATUS_LINE`] bytes, whichever
    /// comes first; the deadline is for all reads together.
    fn read_status_line(&self, stream: &mut TcpStream) -> Result<Vec<u8>, HealthError> {
        let mut line = Vec::with_capacity(MAX_STATUS_LINE);
        let mut chunk = [0u8; MAX_STATUS_LINE];
        while find(&line, CRLF).is_none() && line.len() < MAX_STATUS_LINE {
            stream
                .set_read_timeout(Some(self.remaining()?))
                .map_err(|e| self.fail(e, false))?;
            let room = MAX_STATUS_LINE - line.len();
            match stream.read(&mut chunk[..room]) {
                Ok(0) => break,
                Ok(n) => line.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(self.fail(e, false)),
            }
        }
        Ok(line)
    }

    /// Time left; none left is a timeout (a zero socket timeout would
    /// mean "block forever").
    fn remaining(&self) -> Result<Duration, HealthError> {
        Some(self.deadline.saturating_duration_since(Instant::now()))
            .filter(|left| !left.is_zero())
            .ok_or_else(|| self.timed_out())
    }

    fn timed_out(&self) -> HealthError {
        HealthError::TimedOut {
            addr: self.addr,
            timeout: self.timeout,
        }
    }

    fn fail(&self, source: io::Error, connecting: bool) -> HealthError {
        let addr = self.addr;
        match source.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => self.timed_out(),
            _ if connecting => HealthError::Unreachable { addr, source },
            _ => HealthError::Request { addr, source },
        }
    }
}

/// The status code of `HTTP/1.x <3 digits>` followed by a space or the CRLF,
/// within the first [`MAX_STATUS_LINE`] bytes. Nothing is UTF-8 decoded.
fn parse_status(response: &[u8]) -> Option<u16> {
    let head = &response[..response.len().min(MAX_STATUS_LINE)];
    let line = &head[..find(head, CRLF)?];
    let rest = line
        .strip_prefix(b"HTTP/1.1 ")
        .or_else(|| line.strip_prefix(b"HTTP/1.0 "))?;
    let (code, after) = rest.split_at_checked(STATUS_DIGITS)?;
    let ends_well = after.is_empty() || after.starts_with(b" ");
    (ends_well && code.iter().all(u8::is_ascii_digit)).then(|| {
        code.iter()
            .fold(0, |status, digit| status * 10 + u16::from(digit - b'0'))
    })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests;
