//! The account requests exactly as the official client sends them, for multi-step tests.

use std::net::SocketAddr;

use serde_json::{Value, json};

use crate::fixture::{Body, Request};
use crate::seeds::{SEED_RUSTDESK_ID, SEED_UUID};

/// `deviceInfo` of [`login`].
pub fn device_info() -> Value {
    json!({ "os": "linux", "type": "client", "name": "flow-host" })
}

/// `POST /api/login`, account type: `LoginRequest.toJson`
/// (`flutter/lib/common/hbbs/hbbs.dart:155-177`) with the default
/// `Content-Type: application/json` (`flutter/lib/utils/http_service.dart:18`).
pub fn login(username: &str, password: &str) -> Request {
    let mut request = Request::new("POST", "/api/login");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(json!({
        "username": username, "password": password,
        "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID, "autoLogin": true,
        "type": "account", "deviceInfo": device_info(),
    }));
    request
}

/// `currentUser` / `logout` with a bearer token
/// (`flutter/lib/models/user_model.dart:63-78, 170-183`).
pub fn bearer(path: &str, token: &str) -> Request {
    let mut request = Request::new("POST", path);
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request
        .headers
        .insert("authorization".to_owned(), format!("Bearer {token}"));
    request.body = Body::Json(json!({ "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID }));
    request
}

/// `request` from another TCP peer. Panics on a malformed address (test input).
#[allow(clippy::panic)]
pub fn from(mut request: Request, peer: &str) -> Request {
    let Ok(addr) = peer.parse::<SocketAddr>() else {
        panic!("bad peer address {peer:?}");
    };
    request.peer = Some(addr);
    request
}
