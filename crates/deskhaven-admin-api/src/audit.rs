//! Console audit of admin API operations (06 §7, 04 §7 `console_audit`).
//!
//! **Rule: production write handlers record in their own transaction.** A handler takes
//! [`AuditCtx`] (actor, client ip, request id), passes `ctx.entry(Audit::new(..)...)` to
//! the service, which calls `ConsoleAuditLog::record_in` on its write transaction, and
//! returns [`Audited`] with its response. The change and its row then commit or roll back
//! together, whatever happens to the request afterwards (deadline, client disconnect).
//!
//! **Fallback (safety net)** for a 2xx response without [`Audited`]: the gate records
//! - the handler's [`Audit`] when one is attached, on any route and method (a secret
//!   viewed through a GET, the first-run setup on a `Public` route); and
//! - a default row (actor, operation id as action) for a state-changing request on an
//!   authenticated route that attached nothing.
//!
//! The fallback row is written by a detached task spawned before the gate returns, with
//! no await in between, so neither the request deadline nor a disconnect can cancel it.
//! It never changes the response (the change is committed; a 500 would invite a retry
//! and swallow show-once secrets); if the write fails, the redacted record is logged at
//! ERROR in the request's span (request id), making the log the fallback sink.
//!
//! Actor: the handler's [`Audit::actor`], else the gate's console user, else `system`
//! (logged: a `Public` route's [`Audit`] should name its actor). Failed and denied
//! requests are not recorded here (logins have their own log). Snapshots are redacted by
//! the core log (secret names, credential-shaped values, `deskhaven_config::Secret`).

use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::{IntoResponseParts, ResponseParts};
use deskhaven_core::console_audit::{
    Actor, ConsoleAuditLog, NewConsoleAudit, Refused, TxGuard, redact,
};
use deskhaven_core::state::AppState;
use deskhaven_core::users::Role;
use deskhaven_http::X_REQUEST_ID;
use serde::Serialize;
use serde_json::Value;
use tracing::Instrument;

use crate::AdminApiError;
use crate::domain_error::FORBIDDEN;
use crate::session::{ConsoleUser, client_addr};

/// What an operation changed (or revealed). Every part is optional.
#[derive(Clone, Debug, Default)]
pub struct Audit {
    actor: Option<Actor>,
    action: Option<Cow<'static, str>>,
    target_type: Cow<'static, str>,
    target_id: String,
    before: Option<Value>,
    after: Option<Value>,
}

impl Audit {
    /// `action`: stable machine name, `<resource>.<verb>` (e.g. `user.disable`).
    pub fn new(action: &'static str) -> Self {
        Self {
            action: Some(Cow::Borrowed(action)),
            ..Self::default()
        }
    }

    /// Who acted, when the gate cannot know (a `Public` route, e.g. the setup's new owner).
    #[must_use]
    pub fn actor(self, actor: Actor) -> Self {
        Self {
            actor: Some(actor),
            ..self
        }
    }

    #[must_use]
    pub fn target(self, kind: &'static str, id: impl ToString) -> Self {
        Self {
            target_type: Cow::Borrowed(kind),
            target_id: id.to_string(),
            ..self
        }
    }

    /// Snapshot of the target before the change (redacted when written).
    #[must_use]
    pub fn before(self, snapshot: &impl Serialize) -> Self {
        Self {
            before: Some(snapshot_value(snapshot)),
            ..self
        }
    }

    /// Snapshot of the target after the change (redacted when written).
    #[must_use]
    pub fn after(self, snapshot: &impl Serialize) -> Self {
        Self {
            after: Some(snapshot_value(snapshot)),
            ..self
        }
    }
}

/// Fallback: attached to a response, the gate records it (see the module docs).
impl IntoResponseParts for Audit {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        res.extensions_mut().insert(self);
        Ok(res)
    }
}

/// Response marker: the handler recorded its audit row inside its own transaction; the
/// gate records nothing more.
#[derive(Clone, Copy, Debug)]
pub struct Audited;

impl IntoResponseParts for Audited {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        res.extensions_mut().insert(self);
        Ok(res)
    }
}

/// The facts of the request an audit row needs, for in-transaction recording. Extracting
/// it requires a console session (the gate's, or the cookie on a `Public` route).
#[derive(Clone, Debug)]
pub struct AuditCtx {
    pub actor: Actor,
    pub ip: IpAddr,
    /// `X-Request-Id` of the request, for the handler's own logs.
    pub request_id: Option<String>,
}

impl AuditCtx {
    /// The row for `audit`, attributed to this request (an actor set on `audit` wins).
    pub fn entry(&self, audit: Audit) -> NewConsoleAudit {
        entry(audit, self.actor, self.ip, "")
    }

    /// The [`TxGuard`] of an administrator's write. In the write transaction it re-reads
    /// this request's account and refuses one that is gone, disabled or no longer an admin
    /// or owner (the session's role may be stale by then); `decide` applies the
    /// operation's own rules with the committed role and returns the audit row.
    pub(crate) fn admin_guard<T: 'static>(
        &self,
        log: &ConsoleAuditLog,
        decide: impl FnOnce(Role, Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused>
        + Send
        + 'static,
    ) -> TxGuard<T> {
        let Actor::User(actor_id) = self.actor else {
            return TxGuard::new(log, |_, _| Err(Refused::new(FORBIDDEN)));
        };
        TxGuard::for_account(log, actor_id, move |role, before, after| {
            if role == Role::User {
                return Err(Refused::new(FORBIDDEN));
            }
            decide(role, before, after)
        })
    }
}

impl FromRequestParts<Arc<AppState>> for AuditCtx {
    type Rejection = AdminApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let ConsoleUser(user) = ConsoleUser::from_request_parts(parts, state).await?;
        let ip = client_addr(parts, state).await?.addr;
        let request_id = parts
            .headers
            .get(X_REQUEST_ID)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        Ok(Self {
            actor: Actor::User(user.user_id),
            ip,
            request_id,
        })
    }
}

/// A snapshot that cannot be serialized is recorded as such (and logged), never dropped.
fn snapshot_value(snapshot: &impl Serialize) -> Value {
    serde_json::to_value(snapshot).unwrap_or_else(|err| {
        tracing::error!(error = %err, "audit snapshot is not serializable");
        Value::String("<unserializable snapshot>".to_owned())
    })
}

fn entry(audit: Audit, actor: Actor, ip: IpAddr, default_action: &str) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: audit.actor.unwrap_or(actor),
        action: audit
            .action
            .map_or_else(|| default_action.to_owned(), Cow::into_owned),
        target_type: audit.target_type.into_owned(),
        target_id: audit.target_id,
        before: audit.before,
        after: audit.after,
        ip: Some(ip),
    }
}

/// What the gate knows when it falls back to recording.
pub(crate) struct Fallback<'a> {
    pub user_id: Option<i64>,
    pub ip: IpAddr,
    pub operation_id: &'a str,
}

/// Spawns the fallback write of `audit` (see the module docs). Synchronous on purpose: the
/// task exists once this returns, so the caller cannot be cancelled in between.
pub(crate) fn record_detached(state: &Arc<AppState>, ctx: Fallback<'_>, audit: Audit) {
    let gate_actor = ctx.user_id.map(Actor::User);
    if audit.actor.is_none() && gate_actor.is_none() {
        tracing::warn!(
            operation = ctx.operation_id,
            "audit without an actor; recorded as system"
        );
    }
    let entry = entry(
        audit,
        gate_actor.unwrap_or(Actor::System),
        ctx.ip,
        ctx.operation_id,
    );
    let state = Arc::clone(state);
    let task = async move {
        if let Err(err) = state.console_audit.record(&entry).await {
            log_lost(&entry, &err);
        }
    };
    drop(tokio::spawn(task.in_current_span()));
}

fn log_lost(entry: &NewConsoleAudit, err: &impl std::fmt::Display) {
    let shown = |v: &Option<Value>| v.as_ref().map(|v| redact(v).to_string());
    tracing::error!(
        actor = ?entry.actor,
        action = %entry.action,
        target_type = %entry.target_type,
        target_id = %entry.target_id,
        before = ?shown(&entry.before),
        after = ?shown(&entry.after),
        ip = ?entry.ip,
        error = %err,
        "console audit write failed; the operation was applied but is not in console_audit"
    );
}
