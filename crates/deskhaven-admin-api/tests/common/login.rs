//! Console login helpers (M3-02): requests from an https console origin, cookies taken
//! from `Set-Cookie`, settings changes, TOTP enrolment and codes.

use axum::http::Method;
use deskhaven_admin_api::{CSRF_COOKIE, SESSION_COOKIE};
use deskhaven_config::settings::Change;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::tfa::RecoveryCodes;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::Role;
use serde_json::{Value, json};

use super::{CSRF_HEADER_NAME, Console, Harness, PASSWORD, Reply, request};

/// The console's origin in these tests (https, so the transport is secure).
pub const ORIGIN: &str = "https://console.test";
pub const HOST: &str = "console.test";

/// An enrolled TOTP user: the seed and recovery codes.
pub struct Totp {
    pub secret: TotpSecret,
    pub recovery: Vec<String>,
}

impl Totp {
    /// A code for the step after the current one (inside the ±1 window, and after the
    /// step the enrolment or a previous login recorded).
    pub fn next_code(&self) -> String {
        totp::code_at_step(&self.secret, totp::step_at(SystemClock.now_ms()) + 1)
    }

    pub fn current_code(&self) -> String {
        totp::code_at(&self.secret, SystemClock.now_ms())
    }
}

/// [`super::harness`] with `config` (e.g. a `public_url` for OIDC).
pub async fn harness_with(
    db: &deskhaven_db::testing::TestDb,
    extra: deskhaven_admin_api::AdminRoutes,
    config: &deskhaven_core::state::StateConfig,
) -> Harness {
    use std::sync::Arc;

    let base = super::harness(db, deskhaven_admin_api::AdminRoutes::new()).await;
    let state = Arc::new(deskhaven_core::state::AppState::new(
        db.conn.clone(),
        base.state.settings.clone(),
        Arc::clone(&base.state.crypto),
        "test-signal-key".to_owned(),
        deskhaven_core::users::PasswordHasher::default(),
        Arc::new(SystemClock),
        config,
    ));
    let routes = deskhaven_admin_api::api_routes()
        .unwrap()
        .merge(extra)
        .unwrap();
    let router = deskhaven_http::apply_middleware(
        routes.into_router(Arc::clone(&state)),
        &deskhaven_http::HttpConfig::default(),
        deskhaven_http::ErrorBodies::uniform(deskhaven_admin_api::render_stack_error),
    );
    Harness { state, router }
}

/// [`super::harness`] whose LDAP service talks to `connector` (a fake directory).
pub async fn harness_with_ldap(
    db: &deskhaven_db::testing::TestDb,
    connector: std::sync::Arc<dyn deskhaven_core::ldap::Connector>,
) -> Harness {
    use std::sync::Arc;

    let base = super::harness(db, deskhaven_admin_api::AdminRoutes::new()).await;
    let state = deskhaven_core::state::AppState::new(
        db.conn.clone(),
        base.state.settings.clone(),
        Arc::clone(&base.state.crypto),
        "test-signal-key".to_owned(),
        deskhaven_core::users::PasswordHasher::default(),
        Arc::new(SystemClock),
        &deskhaven_core::state::StateConfig::default(),
    );
    let ldap = deskhaven_core::ldap::LdapService::new(
        state.db.clone(),
        Arc::clone(&state.crypto),
        state.users.clone(),
        state.groups.clone(),
        Arc::clone(&state.clock),
        connector,
    );
    let state = Arc::new(deskhaven_core::state::AppState { ldap, ..state });
    let routes = deskhaven_admin_api::api_routes().unwrap();
    let router = deskhaven_http::apply_middleware(
        routes.into_router(Arc::clone(&state)),
        &deskhaven_http::HttpConfig::default(),
        deskhaven_http::ErrorBodies::uniform(deskhaven_admin_api::render_stack_error),
    );
    Harness { state, router }
}

impl Harness {
    /// A state-changing request from the console origin; `console` adds its cookie and
    /// CSRF token, else the anonymous CSRF value is sent.
    pub async fn post(&self, path: &str, console: Option<&Console>, body: Value) -> Reply {
        self.post_with(path, console, body, &[]).await
    }

    pub async fn post_with(
        &self,
        path: &str,
        console: Option<&Console>,
        body: Value,
        extra: &[(&str, &str)],
    ) -> Reply {
        let mut headers = vec![("origin", ORIGIN), ("host", HOST)];
        match console {
            Some(c) => {
                headers.push(("cookie", c.cookie.as_str()));
                headers.push((CSRF_HEADER_NAME, c.csrf.as_str()));
            }
            None => headers.push((CSRF_HEADER_NAME, "anonymous")),
        }
        headers.extend_from_slice(extra);
        self.raw(request(Method::POST, path, &headers, Some(body)))
            .await
    }

    /// `POST /auth/login`.
    pub async fn login(&self, username: &str, password: &str) -> Reply {
        let body = json!({ "username": username, "password": password });
        self.post("/admin/api/v1/auth/login", None, body).await
    }

    /// A password login that must sign in; the console of the new session.
    pub async fn signed_in(&self, username: &str) -> Console {
        let reply = self.login(username, PASSWORD).await;
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        assert_eq!(reply.body["status"], "signed_in", "{:?}", reply.body);
        console_of(&reply)
    }

    /// An active local user of `role` with [`PASSWORD`].
    pub async fn user(&self, username: &str, role: Role) -> i64 {
        self.create_user(username, role).await
    }

    /// Enrols TOTP for `user_id` directly through the service.
    pub async fn enrol(&self, user_id: i64) -> Totp {
        let enrolment = self.state.tfa.begin_enrolment(user_id).await.unwrap();
        let secret = TotpSecret::from_base32(&enrolment.secret).unwrap();
        let code = totp::code_at(&secret, SystemClock.now_ms());
        let codes: RecoveryCodes = self
            .state
            .tfa
            .confirm_enrolment(user_id, &code)
            .await
            .unwrap();
        Totp {
            secret,
            recovery: codes.codes().map(str::to_owned).collect(),
        }
    }

    /// Sets the setting `key`, whatever its version.
    pub async fn set_setting(&self, key: &str, value: Value) {
        let settings = &self.state.settings;
        let version = settings.snapshot().view(key).unwrap().version;
        let change = Change {
            key: key.to_owned(),
            value,
            version,
        };
        settings
            .update(UpdateRequest {
                changes: vec![change],
                actor: None,
                reason: None,
            })
            .await
            .unwrap();
    }
}

/// The `Set-Cookie` values of a reply.
pub fn set_cookies(reply: &Reply) -> Vec<String> {
    reply
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

/// The value `name` is set to in `reply` (`None`: not set).
pub fn cookie_value(reply: &Reply, name: &str) -> Option<String> {
    set_cookies(reply).iter().find_map(|line| {
        let pair = line.split(';').next()?;
        let (found, value) = pair.split_once('=')?;
        (found == name).then(|| value.to_owned())
    })
}

/// The console session a signed-in reply set.
pub fn console_of(reply: &Reply) -> Console {
    let token = cookie_value(reply, SESSION_COOKIE).unwrap_or_else(|| panic!("no session cookie"));
    let csrf = cookie_value(reply, CSRF_COOKIE).unwrap_or_else(|| panic!("no CSRF cookie"));
    assert!(!token.is_empty(), "{:?}", set_cookies(reply));
    let user_id = reply.body["user"]["id"]
        .as_i64()
        .or_else(|| reply.body["id"].as_i64())
        .unwrap_or_default();
    Console {
        user_id,
        cookie: format!("{SESSION_COOKIE}={token}"),
        csrf,
    }
}
