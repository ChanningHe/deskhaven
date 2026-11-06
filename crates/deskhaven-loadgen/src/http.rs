//! One HTTP/1.1 request per TCP connection from a chosen source address, like the client's
//! `post_request`, which builds a new HTTP client for every call (`src/common.rs`).

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, header};
use hyper_util::rt::TokioIo;
use tokio::net::TcpSocket;

use crate::stats::Failure;

/// Status and body of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Bytes,
}

/// Sends `method path` with an optional JSON body to `server` from `local` and reads the
/// whole answer, all within `limit` (connecting included).
pub async fn send(
    local: IpAddr,
    server: SocketAddr,
    method: Method,
    path: &str,
    json: Option<String>,
    limit: Duration,
) -> Result<Reply, Failure> {
    let request = build(server, method, path, json)?;
    tokio::time::timeout(limit, exchange(local, server, request))
        .await
        .map_err(|_| Failure::Timeout)?
}

fn build(
    server: SocketAddr,
    method: Method,
    path: &str,
    json: Option<String>,
) -> Result<Request<Full<Bytes>>, Failure> {
    let builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, server.to_string());
    let (builder, body) = match json {
        // `post_request` always sets this content type (`common.rs:1760`).
        Some(json) => (
            builder.header(header::CONTENT_TYPE, "application/json"),
            Bytes::from(json),
        ),
        None => (builder, Bytes::new()),
    };
    builder
        .body(Full::new(body))
        .map_err(|err| Failure::Transport(format!("request: {err}")))
}

async fn exchange(
    local: IpAddr,
    server: SocketAddr,
    request: Request<Full<Bytes>>,
) -> Result<Reply, Failure> {
    let transport =
        |what: &str, err: &dyn std::fmt::Display| Failure::Transport(format!("{what}: {err}"));
    let socket = match local {
        IpAddr::V4(_) => TcpSocket::new_v4(),
        IpAddr::V6(_) => TcpSocket::new_v6(),
    }
    .map_err(|err| transport("socket", &err))?;
    socket
        .bind(SocketAddr::new(local, 0))
        .map_err(|err| transport("bind", &err))?;
    let stream = socket
        .connect(server)
        .await
        .map_err(|err| transport("connect", &err))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|err| transport("handshake", &err))?;
    let call = async move {
        let response = sender.send_request(request).await?;
        let status = response.status().as_u16();
        let body = response.into_body().collect().await?.to_bytes();
        Ok::<_, hyper::Error>(Reply { status, body })
        // `sender` drops here, which lets `connection` finish.
    };
    // A connection error also fails the call, which is what gets reported.
    let (reply, _connection) = tokio::join!(call, connection);
    reply.map_err(|err| transport("http", &err))
}
