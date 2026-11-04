//! `POST /ldap-tests/connection`, `POST /ldap-tests/user`: the login steps one by one, for
//! a saved directory or an unsaved form. Owner only: the target is whatever URL the
//! request names. Guards:
//!
//! - **Address filter**: a form goes through `DirectorySettings::from_new`, the same URL
//!   rules as a saved directory (TLS, no link-local or metadata literals, no numeric host
//!   spellings); a saved directory was checked when written, and every connection resolves
//!   and filters host names the same way. Nothing is contacted for a refused form.
//! - **Rate limit**: every test spends one token of the client's login-type IP bucket
//!   (`auth.ip_rate_limit.per_minute`); the core holds a directory slot shared with logins
//!   and ends a test after a total timeout.
//! - **Password oracle** (`test_user` with a password): a username a login would refuse as
//!   malformed is a 422 before anything is reserved or contacted (it would charge the
//!   shared malformed lockout row while the directory matched a real entry). Otherwise the
//!   test is reserved against the username like a login (`LoginThrottle::pre_check`: 429
//!   while it is locked) before the directory sees it, then settled as a login would be
//!   ([`Diagnostic::attempted`]): counted once the password reached the directory (refused,
//!   interrupted, timed out) or the directory had no single entry for the name; refunded
//!   when the bind passed or the directory failed before that. A passed test is no login:
//!   it does not clear earlier failures.
//!
//! Tests are recorded in the console audit (the directory or the form's connection, the
//! tested username, the result; never a password). Nothing else is written.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::auth::limiter::{Attempt, ClientAddr, ThrottleError};
use deskhaven_core::ldap::{Diagnostic, DirectorySettings, NewDirectory, Step, StepStatus};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Password, username_norm};
use serde_json::{Map, Value, json};

use super::diagnostics_view::{LdapConnectionTest, LdapDiagnosticView, LdapUserTest};
use super::ldap_view::NewLdapDirectoryBody;
use crate::domain_error::invalid_field;
use crate::session::Client;
use crate::{AdminApiError, Audit, ValidJson};

type Problem = crate::ErrorEnvelope<'static>;

/// Tests connect and service bind (audited `ldap_directory.test_connection`).
#[utoipa::path(
    post,
    path = "/ldap-tests/connection",
    tag = "identity",
    operation_id = "test_ldap_connection",
    security(("console_session" = ["owner"])),
    request_body = LdapConnectionTest,
    responses(
        (status = 200, description = "The result of every step", body = LdapDiagnosticView),
        (status = 404, description = "No such directory", body = Problem),
        (status = 422, description = "Not exactly one target, or an invalid form", body = Problem),
        (status = 429, description = "Too many tests from this client (`rate_limited`)", body = Problem),
    )
)]
pub async fn test_ldap_connection(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    ValidJson(body): ValidJson<LdapConnectionTest>,
) -> Result<(Audit, Json<LdapDiagnosticView>), AdminApiError> {
    let target = Target::of(body.directory_id, body.directory)?;
    let settings = target.settings(&state).await?;
    spend_token(&state, client)?;
    let diagnostic = state.ldap.test_connection(&settings).await;
    tracing::info!(directory = %settings.name(), passed = diagnostic.passed(), "LDAP connection test");
    let outcome = json!({ "passed": diagnostic.passed() });
    let audit = target.audit("ldap_directory.test_connection", &settings, outcome);
    Ok((audit, Json(diagnostic.into())))
}

/// Tests every login step for a username (audited `ldap_directory.test_user`). With a
/// password this is a password check: counted against the username's lockout when it
/// fails, refused with 429 while the username is locked.
#[utoipa::path(
    post,
    path = "/ldap-tests/user",
    tag = "identity",
    operation_id = "test_ldap_user",
    security(("console_session" = ["owner"])),
    request_body = LdapUserTest,
    responses(
        (status = 200, description = "The result of every step", body = LdapDiagnosticView),
        (status = 404, description = "No such directory", body = Problem),
        (status = 422, description = "Not exactly one target, an invalid form, or a malformed username", body = Problem),
        (status = 429, description = "Too many tests from this client, or the username is locked (`rate_limited`)", body = Problem),
    )
)]
pub async fn test_ldap_user(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    ValidJson(body): ValidJson<LdapUserTest>,
) -> Result<(Audit, Json<LdapDiagnosticView>), AdminApiError> {
    // A login reads such a name as malformed: never let it reach the directory as if valid.
    if username_norm(&body.username).is_none() {
        let reason = "must be 1-64 characters of A-Z a-z 0-9 . _ @ -";
        return Err(invalid_field("username", reason));
    }
    let target = Target::of(body.directory_id, body.directory)?;
    let settings = target.settings(&state).await?;
    let password = body
        .password
        .filter(|p| !p.is_empty())
        .map(|p| Password::new(p.as_str()));
    let attempt = reserve(&state, client, &body.username, password.is_some()).await?;
    let diagnostic = state
        .ldap
        .test_user(&settings, &body.username, password.as_ref())
        .await;
    if let Some(attempt) = attempt {
        settle(&state, attempt, &diagnostic).await;
    }
    tracing::info!(directory = %settings.name(), passed = diagnostic.passed(), "LDAP user test");
    let outcome = json!({
        "username": body.username,
        "user_bind": password.is_some(),
        "passed": diagnostic.passed(),
    });
    let audit = target.audit("ldap_directory.test_user", &settings, outcome);
    Ok((audit, Json(diagnostic.into())))
}

/// What a test runs against.
enum Target {
    Saved(i64),
    Unsaved(Box<NewDirectory>),
}

impl Target {
    fn of(id: Option<i64>, form: Option<NewLdapDirectoryBody>) -> Result<Self, AdminApiError> {
        match (id, form) {
            (Some(id), None) => Ok(Self::Saved(id)),
            (None, Some(form)) => Ok(Self::Unsaved(Box::new(form.into()))),
            _ => Err(invalid_field(
                "directory",
                "give exactly one of directory_id and directory",
            )),
        }
    }

    /// The settings to test: the stored directory with its password opened, or the form
    /// through the same validation as a saved directory.
    async fn settings(&self, state: &AppState) -> Result<DirectorySettings, AdminApiError> {
        match self {
            Self::Saved(id) => Ok(state.ldap.stored_settings(*id).await?),
            Self::Unsaved(form) => Ok(DirectorySettings::from_new(form)?),
        }
    }

    /// The audit row of a test of this target (`details`: the outcome).
    fn audit(&self, action: &'static str, settings: &DirectorySettings, details: Value) -> Audit {
        let (id, connection) = match self {
            Self::Saved(id) => (id.to_string(), Value::Null),
            Self::Unsaved(form) => ("new".to_owned(), form_connection(form)),
        };
        let directory = [("directory".to_owned(), json!(settings.name()))];
        let after: Map<String, Value> = directory
            .into_iter()
            .chain(members(connection))
            .chain(members(details))
            .collect();
        Audit::new(action)
            .target("ldap_directory", id)
            .after(&after)
    }
}

fn spend_token(state: &AppState, client: ClientAddr) -> Result<(), AdminApiError> {
    state
        .login_throttle
        .ip()
        .check(client)
        .map_err(AdminApiError::rate_limited)
}

/// With a password: one login attempt for `username` (takes the IP token too); without:
/// the IP token only.
async fn reserve(
    state: &AppState,
    client: ClientAddr,
    username: &str,
    with_password: bool,
) -> Result<Option<Attempt>, AdminApiError> {
    if !with_password {
        spend_token(state, client)?;
        return Ok(None);
    }
    match state.login_throttle.pre_check(client, username).await {
        Ok(attempt) => Ok(Some(attempt)),
        Err(ThrottleError::Throttled { retry_after }) => {
            Err(AdminApiError::rate_limited(retry_after))
        }
        Err(ThrottleError::Db(err)) => Err(AdminApiError::internal(err)),
    }
}

/// Settles a reserved attempt by the outcome (see the module docs).
async fn settle(state: &AppState, attempt: Attempt, diagnostic: &Diagnostic) {
    if is_counted(diagnostic) {
        state.login_throttle.on_failure(attempt);
        return;
    }
    if let Err(err) = state.login_throttle.refund(attempt).await {
        tracing::warn!(error = %err, "cannot refund an LDAP test attempt");
    }
}

/// A login would have counted it: attempted, and the password not accepted.
fn is_counted(diagnostic: &Diagnostic) -> bool {
    let accepted = diagnostic
        .steps
        .iter()
        .any(|s| s.step == Step::UserBind && s.status == StepStatus::Passed);
    diagnostic.attempted && !accepted
}

/// The non-secret connection of an unsaved form, for its audit row.
fn form_connection(form: &NewDirectory) -> Value {
    json!({
        "url": form.url,
        "start_tls": form.start_tls,
        "tls_verify": form.tls_verify,
        "bind_dn": form.bind_dn,
    })
}

fn members(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(members) => members,
        _ => Map::new(),
    }
}
