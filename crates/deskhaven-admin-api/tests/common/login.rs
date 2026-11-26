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

/// https, so the transport is secure.
pub const ORIGIN: &str = "https://console.test";
pub const HOST: &str = "console.test";

pub struct Totp {
    pub secret: TotpSecret,
    pub recovery: Vec<String>,
}

impl Totp {
    /// Inside the ±1 window and after the step the enrolment or a previous login recorded.
    pub fn next_code(&self) -> String {
        totp::code_at_step(&self.secret, totp::step_at(SystemClock.now_ms()) + 1)
    }

    pub fn current_code(&self) -> String {
        totp::code_at(&self.secret, SystemClock.now_ms())
    }
}

pub async fn harness_with(
    db: &deskhaven_db::testing::TestDb,
    extra: deskhaven_admin_api::AdminRoutes,
    config: &deskhaven_core::state::StateConfig,
) -> Harness {
    use std::sync::Arc;

    let base = super::harness(db, deskhaven_admin_api::AdminRoutes::new()).await;
    let state = Arc::new(super::offline(deskhaven_core::state::AppState::new(
        db.conn.clone(),
        base.state.settings.clone(),
        Arc::clone(&base.state.crypto),
        "test-signal-key".to_owned(),
        deskhaven_core::users::PasswordHasher::default(),
        Arc::new(SystemClock),
        config,
    )));
    let routes = deskhaven_admin_api::api_routes()
        .unwrap()
        .merge(extra)
        .unwrap();
    let router = deskhaven_http::apply_middleware(
        routes.into_router(Arc::clone(&state)),
        &deskhaven_http::HttpConfig::default(),
        deskhaven_http::ErrorBodies::uniform(deskhaven_admin_api::render_stack_error),
    );
    Harness::new(state, router)
}

pub async fn harness_with_ldap(
    db: &deskhaven_db::testing::TestDb,
    connector: std::sync::Arc<dyn deskhaven_core::ldap::Connector>,
) -> Harness {
    let limits = deskhaven_core::ldap::DirectoryLimits::default();
    harness_with_ldap_limits(db, connector, limits).await
}

pub async fn harness_with_ldap_limits(
    db: &deskhaven_db::testing::TestDb,
    connector: std::sync::Arc<dyn deskhaven_core::ldap::Connector>,
    limits: deskhaven_core::ldap::DirectoryLimits,
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
    )
    .with_limits(limits);
    let state = Arc::new(super::offline(deskhaven_core::state::AppState {
        ldap,
        ..state
    }));
    let routes = deskhaven_admin_api::api_routes().unwrap();
    let router = deskhaven_http::apply_middleware(
        routes.into_router(Arc::clone(&state)),
        &deskhaven_http::HttpConfig::default(),
        deskhaven_http::ErrorBodies::uniform(deskhaven_admin_api::render_stack_error),
    );
    Harness::new(state, router)
}

impl Harness {
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

    pub async fn login(&self, username: &str, password: &str) -> Reply {
        let body = json!({ "username": username, "password": password });
        self.post("/admin/api/v1/auth/login", None, body).await
    }

    pub async fn signed_in(&self, username: &str) -> Console {
        let reply = self.login(username, PASSWORD).await;
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        assert_eq!(reply.body["status"], "signed_in", "{:?}", reply.body);
        console_of(&reply)
    }

    pub async fn user(&self, username: &str, role: Role) -> i64 {
        self.create_user(username, role).await
    }

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

pub fn set_cookies(reply: &Reply) -> Vec<String> {
    reply
        .headers
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_owned())
        .collect()
}

pub fn cookie_value(reply: &Reply, name: &str) -> Option<String> {
    set_cookies(reply).iter().find_map(|line| {
        let pair = line.split(';').next()?;
        let (found, value) = pair.split_once('=')?;
        (found == name).then(|| value.to_owned())
    })
}

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
