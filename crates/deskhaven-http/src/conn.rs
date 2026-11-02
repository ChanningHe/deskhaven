//! Accept loop for one listener on hyper's HTTP/1.1 server (`axum::serve` sets
//! no timer, so it has no header-read timeout; review H2).
//!
//! - a client must send a complete request head within `header_read_timeout`
//!   (hyper keeps that timer running while a keep-alive connection is idle) and
//!   it may not exceed `max_header_bytes` / `max_headers`;
//! - a response write that makes no progress for `write_stall_timeout` closes
//!   the connection ([`crate::io`]);
//! - at most `max_connections` are served at once (the rest wait in the
//!   backlog), and at most `max_connections_per_ip` per untrusted address
//!   (excess connections are closed at once, [`crate::per_ip`]);
//! - on shutdown: stop accepting, let in-flight requests finish, and drop whatever
//!   is still open after `drain_timeout`.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use axum::http::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

use crate::config::HttpConfig;
use crate::io::WriteStallTimeout;
use crate::per_ip::{PerIpLimiter, Slot};

/// Pause after `accept` fails for lack of resources before retrying.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);
/// hyper panics on a smaller read buffer.
const MIN_BUF_SIZE: usize = 8 * 1024;

struct Server {
    name: &'static str,
    router: Router,
    builder: http1::Builder,
    write_stall_timeout: Duration,
    per_ip: PerIpLimiter,
    graceful: GracefulShutdown,
    conns: JoinSet<()>,
}

fn http1_builder(cfg: &HttpConfig) -> http1::Builder {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(cfg.header_read_timeout)
        .max_buf_size(cfg.max_header_bytes.max(MIN_BUF_SIZE))
        .max_header_size(cfg.max_header_bytes)
        .max_headers(cfg.max_headers)
        .keep_alive(true);
    builder
}

pub(crate) async fn serve_listener(
    name: &'static str,
    listener: TcpListener,
    router: Router,
    cfg: &HttpConfig,
    shutdown: CancellationToken,
) {
    let mut server = Server {
        name,
        router,
        builder: http1_builder(cfg),
        write_stall_timeout: cfg.write_stall_timeout,
        per_ip: PerIpLimiter::new(cfg.max_connections_per_ip, cfg.trusted_proxies.clone()),
        graceful: GracefulShutdown::new(),
        conns: JoinSet::new(),
    };
    let max_connections = cfg.max_connections.max(1);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            Some(joined) = server.conns.join_next(), if !server.conns.is_empty() => {
                log_join(name, joined);
            }
            accepted = listener.accept(), if server.conns.len() < max_connections => {
                match accepted {
                    Ok((stream, peer)) => server.admit(stream, peer),
                    Err(err) => accept_failed(name, &err).await,
                }
            }
        }
    }
    drop(listener);
    server.drain(cfg.drain_timeout).await;
}

impl Server {
    /// Serve the connection, or close it at once if its address is over the cap.
    fn admit(&mut self, stream: TcpStream, peer: SocketAddr) {
        match self.per_ip.try_acquire(peer.ip()) {
            Some(slot) => self.spawn(stream, peer, slot),
            None => drop(stream),
        }
    }

    fn spawn(&mut self, stream: TcpStream, peer: SocketAddr, slot: Slot) {
        // Same contract as axum's `into_make_service_with_connect_info`.
        let service = self
            .router
            .clone()
            .map_request(move |mut req: Request<Incoming>| {
                req.extensions_mut().insert(ConnectInfo(peer));
                req
            });
        let io = TokioIo::new(WriteStallTimeout::new(stream, self.write_stall_timeout));
        let conn = self
            .builder
            .serve_connection(io, TowerToHyperService::new(service));
        let watched = self.graceful.watch(conn);
        let name = self.name;
        self.conns.spawn(async move {
            let _slot = slot;
            if let Err(err) = watched.await {
                tracing::debug!(listener = name, %peer, error = %err, "connection closed with error");
            }
        });
    }

    async fn drain(mut self, deadline: Duration) {
        let open = self.conns.len();
        if tokio::time::timeout(deadline, self.graceful.shutdown())
            .await
            .is_err()
        {
            tracing::warn!(
                listener = self.name,
                open,
                "drain deadline exceeded; dropping remaining connections"
            );
        }
        self.conns.shutdown().await;
    }
}

fn log_join(name: &'static str, joined: Result<(), JoinError>) {
    if let Err(err) = joined {
        tracing::error!(listener = name, error = %err, "connection task failed");
    }
}

/// Out of descriptors/buffers/memory: retrying at once would spin. Anything
/// else (`ECONNABORTED`, `ECONNRESET`, `EPROTO`, ...) concerns one connection.
fn is_resource_exhaustion(err: &io::Error) -> bool {
    if err.kind() == io::ErrorKind::OutOfMemory {
        return true;
    }
    #[cfg(unix)]
    {
        use rustix::io::Errno;
        Errno::from_io_error(err)
            .is_some_and(|e| [Errno::MFILE, Errno::NFILE, Errno::NOBUFS, Errno::NOMEM].contains(&e))
    }
    #[cfg(not(unix))]
    {
        true
    }
}

async fn accept_failed(name: &'static str, err: &io::Error) {
    if !is_resource_exhaustion(err) {
        tracing::debug!(listener = name, error = %err, "accept failed for one connection");
        return;
    }
    tracing::warn!(listener = name, error = %err, "accept failed: out of resources; backing off");
    tokio::time::sleep(ACCEPT_BACKOFF).await;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rustix::io::Errno;

    fn os_error(errno: Errno) -> io::Error {
        io::Error::from_raw_os_error(errno.raw_os_error())
    }

    #[test]
    fn resource_errors_back_off() {
        for errno in [Errno::MFILE, Errno::NFILE, Errno::NOBUFS, Errno::NOMEM] {
            assert!(is_resource_exhaustion(&os_error(errno)), "{errno:?}");
        }
    }

    #[test]
    fn per_connection_errors_do_not() {
        for errno in [
            Errno::CONNABORTED,
            Errno::CONNRESET,
            Errno::PROTO,
            Errno::PERM,
        ] {
            assert!(!is_resource_exhaustion(&os_error(errno)), "{errno:?}");
        }
    }
}
