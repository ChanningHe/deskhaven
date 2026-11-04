//! Self-service helpers (M3-13): requests from the https console origin with any method.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{Method, StatusCode};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::Role;
use deskhaven_db::entity::user::{self, AuthSource};
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde_json::{Value, json};
use tower::ServiceExt;

use super::login::{HOST, ORIGIN, console_of};
use super::{CSRF_HEADER_NAME, Console, Harness, PASSWORD, Reply, request};

pub const ME: &str = "/admin/api/v1/me";
pub const ACCOUNT: &str = "/admin/api/v1/me/account";
pub const SESSIONS: &str = "/admin/api/v1/me/sessions";
pub const REVOKE_OTHERS: &str = "/admin/api/v1/me/sessions/revoke-others";
pub const DEVICES: &str = "/admin/api/v1/me/devices";
pub const BOOKS: &str = "/admin/api/v1/me/address-books";
pub const ENROL: &str = "/admin/api/v1/me/totp/enrolment";
pub const CONFIRM: &str = "/admin/api/v1/me/totp/enrolment/confirm";
pub const DISABLE: &str = "/admin/api/v1/me/totp/disable";
pub const RECOVERY: &str = "/admin/api/v1/me/totp/recovery-codes";

/// A user enrolled through the self-service endpoints.
pub struct Enrolled {
    pub secret: TotpSecret,
    pub recovery: Vec<String>,
    /// The console session the confirmation handed out (the old one is rotated away).
    pub console: Console,
}

impl Enrolled {
    /// A code of the step after the current one: inside the accepted window and after the
    /// step the enrolment recorded. Only one such code can be spent.
    pub fn next_code(&self) -> String {
        totp::code_at_step(&self.secret, totp::step_at(SystemClock.now_ms()) + 1)
    }
}

/// Signs `username` in with its password, then turns the account into a `source` account
/// (no local password) and drops the cached session principal.
pub async fn external_console(h: &Harness, username: &str, source: AuthSource) -> Console {
    let id = h.create_user(username, Role::User).await;
    let console = h.signed_in(username).await;
    let row = user::ActiveModel {
        id: Set(id),
        auth_source: Set(source),
        password_hash: Set(None),
        ..Default::default()
    };
    row.update(&h.state.db).await.unwrap();
    h.state.sessions.cache().flush();
    console
}

impl Harness {
    /// `method path` from the console origin as `console` (cookie and CSRF token).
    pub async fn as_console(
        &self,
        method: Method,
        path: &str,
        console: &Console,
        body: Option<Value>,
    ) -> Reply {
        let headers = [
            ("origin", ORIGIN),
            ("host", HOST),
            ("cookie", console.cookie.as_str()),
            (CSRF_HEADER_NAME, console.csrf.as_str()),
        ];
        self.raw(request(method, path, &headers, body)).await
    }

    /// `method path` from the console origin with no session (the anonymous CSRF value, as
    /// the login page sends it).
    pub async fn as_anonymous(&self, method: Method, path: &str, body: Option<Value>) -> Reply {
        let headers = [
            ("origin", ORIGIN),
            ("host", HOST),
            (CSRF_HEADER_NAME, "anonymous"),
        ];
        self.raw(request(method, path, &headers, body)).await
    }

    /// Enables TOTP through the self-service endpoints (begin, then confirm).
    pub async fn enrol_through_console(&self, console: &Console) -> Enrolled {
        self.enrol_with_password(console, PASSWORD).await
    }

    /// [`enrol_through_console`](Self::enrol_through_console) for an account whose current
    /// password is `password` (a directory account's differs).
    pub async fn enrol_with_password(&self, console: &Console, password: &str) -> Enrolled {
        let begun = self.post(ENROL, Some(console), json!({})).await;
        assert_eq!(begun.status, 200, "{:?}", begun.body);
        let secret = TotpSecret::from_base32(begun.body["secret"].as_str().unwrap()).unwrap();
        let code = totp::code_at(&secret, SystemClock.now_ms());
        let body = json!({ "current_password": password, "code": code });
        let done = self.post(CONFIRM, Some(console), body).await;
        assert_eq!(done.status, 200, "{:?}", done.body);
        let recovery = done.body["recovery_codes"].as_array().unwrap();
        Enrolled {
            secret,
            recovery: recovery
                .iter()
                .map(|c| c.as_str().unwrap().to_owned())
                .collect(),
            console: console_of(&done),
        }
    }
}

/// `POST /api/login` of the official client API with `body`; status and JSON reply.
pub async fn client_login(h: &Harness, body: Value) -> (StatusCode, Value) {
    let router = apply_middleware(
        deskhaven_client_api::router().with_state(Arc::clone(&h.state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_client_api::render_stack_error),
    );
    let response = router
        .oneshot(request(Method::POST, "/api/login", &[], Some(body)))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// The first step of a client login: username, password and a device.
pub fn client_password_step(username: &str, password: &str) -> Value {
    json!({
        "type": "account", "username": username, "password": password,
        "id": "123456789", "uuid": "uuid-1", "deviceInfo": { "os": "linux", "name": "box" },
    })
}

/// The second step answering `challenge` (the first step's reply) with `code`.
pub fn client_code_step(challenge: &Value, code: &str) -> Value {
    json!({
        "type": "email_code", "username": challenge["user"]["name"],
        "secret": challenge["secret"], "verificationCode": code, "tfaCode": code,
        "id": "123456789", "uuid": "uuid-1", "deviceInfo": { "os": "linux", "name": "box" },
    })
}
