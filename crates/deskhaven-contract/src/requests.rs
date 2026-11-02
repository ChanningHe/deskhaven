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

/// `POST /api/login`, second step of a TOTP login: what `verificationCodeDialog`
/// sends (`flutter/lib/common/widgets/login.dart:1016-1024`, serialized by
/// `hbbs.dart:155-177`): the code in both `verificationCode` and `tfaCode`,
/// the challenge `secret`, the first reply's `user.name` as `username`, no
/// password.
pub fn second_step(secret: &str, username: &str, code: &str) -> Request {
    let mut request = Request::new("POST", "/api/login");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(json!({
        "verificationCode": code, "tfaCode": code, "secret": secret,
        "username": username, "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID,
        "autoLogin": true, "type": "email_code", "deviceInfo": device_info(),
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

/// `POST /api/oidc/auth` as `OidcSession::auth` sends it (`src/hbbs_http/account.rs:160-176`,
/// `post_request` always sets `Content-Type: application/json`, `src/common.rs:1760`).
pub fn oidc_auth(op: &str) -> Request {
    let mut request = Request::new("POST", "/api/oidc/auth");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(json!({
        "op": op, "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID,
        "deviceInfo": device_info(), "apiDomain": "http://127.0.0.1:21114",
    }));
    request
}

/// `GET /api/oidc/auth-query` as `OidcSession::query` builds it
/// (`src/hbbs_http/account.rs:179-198`: `Url::parse_with_params`, so form-urlencoded).
pub fn oidc_query(code: &str, id: &str, uuid: &str) -> Request {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", code)
        .append_pair("id", id)
        .append_pair("uuid", uuid)
        .finish();
    Request::new("GET", &format!("/api/oidc/auth-query?{query}"))
}

/// The browser's redirect back from the IdP with these query pairs.
pub fn oidc_callback(pairs: &[(&str, &str)]) -> Request {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        query.append_pair(key, value);
    }
    Request::new("GET", &format!("/api/oidc/callback?{}", query.finish()))
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
