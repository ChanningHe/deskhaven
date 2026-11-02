//! Accept loop for one listener on hyper's HTTP/1.1 server (`axum::serve` sets
//! no timer, so it has no header-read timeout; review H2).
//!
//! - a client must send a complete request head within `header_read_timeout`;
//!   hyper keeps that timer running while a keep-alive connection is idle;
//! - at most `max_connections` are served at once, the rest wait in the backlog;
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

use crate::middleware::HttpConfig;

/// Pause after a failed `accept` (e.g. out of file descriptors) before retrying.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

struct Server {
    name: &'static str,
    router: Router,
    builder: http1::Builder,
    graceful: GracefulShutdown,
    conns: JoinSet<()>,
}

pub(crate) async fn serve_listener(
    name: &'static str,
    listener: TcpListener,
    router: Router,
    cfg: &HttpConfig,
    shutdown: CancellationToken,
) {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(cfg.header_read_timeout)
        .keep_alive(true);
    let mut server = Server {
        name,
        router,
        builder,
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
                    Ok((stream, peer)) => server.spawn(stream, peer),
                    Err(err) => accept_failed(name, &err).await,
                }
            }
        }
    }
    drop(listener);
    server.drain(cfg.drain_timeout).await;
}

impl Server {
    fn spawn(&mut self, stream: TcpStream, peer: SocketAddr) {
        // Same contract as axum's `into_make_service_with_connect_info`.
        let service = self
            .router
            .clone()
            .map_request(move |mut req: Request<Incoming>| {
                req.extensions_mut().insert(ConnectInfo(peer));
                req
            });
        let conn = self
            .builder
            .serve_connection(TokioIo::new(stream), TowerToHyperService::new(service));
        let watched = self.graceful.watch(conn);
        let name = self.name;
        self.conns.spawn(async move {
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

async fn accept_failed(name: &'static str, err: &io::Error) {
    tracing::warn!(listener = name, error = %err, "accept failed");
    tokio::time::sleep(ACCEPT_BACKOFF).await;
}
