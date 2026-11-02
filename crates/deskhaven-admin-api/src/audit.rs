//! Console audit of admin API writes (06 §7, 04 §7 `console_audit`).
//!
//! The gate writes one row for every **successful** (2xx) state-changing request on an
//! authenticated route, after the handler returned: actor = the console user, ip = the
//! resolved client address. What changed comes from the handler, which adds an [`Audit`]
//! to its response (`(Audit::new("user.update").target("user", id).before(&old)
//! .after(&new), Json(body))`); without one the row still records the actor and the
//! operation id as action. Snapshots are redacted by the core log (secret field names
//! plus `deskhaven_config::Secret` values; see `deskhaven_core::console_audit`).
//! Denied, failed and read-only requests are not recorded here (logins have their own
//! log; `Public` routes have no actor and record their own rows if they need one).
//!
//! **Failure mode**: the handler's change is already committed when the row is written,
//! so an audit failure cannot undo it. The response is replaced by a 500 (the operator
//! sees that something is wrong instead of an apparently clean, unaudited change) and the
//! redacted record is logged at ERROR with the request id, making the log the fallback
//! audit sink. Clients must treat that 500 like any other: the change may have happened.

use std::borrow::Cow;
use std::net::IpAddr;

use axum::response::{IntoResponseParts, ResponseParts};
use deskhaven_core::console_audit::{Actor, NewConsoleAudit, redact};
use deskhaven_core::state::AppState;
use serde::Serialize;
use serde_json::Value;

use crate::AdminApiError;

/// What a write changed, attached to the handler's response. Every part is optional.
#[derive(Clone, Debug, Default)]
pub struct Audit {
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

impl IntoResponseParts for Audit {
    type Error = std::convert::Infallible;

    fn into_response_parts(self, mut res: ResponseParts) -> Result<ResponseParts, Self::Error> {
        res.extensions_mut().insert(self);
        Ok(res)
    }
}

/// A snapshot that cannot be serialized is recorded as such (and logged), never dropped.
fn snapshot_value(snapshot: &impl Serialize) -> Value {
    serde_json::to_value(snapshot).unwrap_or_else(|err| {
        tracing::error!(error = %err, "audit snapshot is not serializable");
        Value::String("<unserializable snapshot>".to_owned())
    })
}

/// The request facts the gate knows; the handler's [`Audit`] fills in the rest.
pub(crate) struct AuditContext<'a> {
    pub user_id: i64,
    pub ip: IpAddr,
    pub operation_id: &'a str,
}

/// Writes the row; on failure logs the redacted record and returns the 500 to send.
pub(crate) async fn record(
    state: &AppState,
    ctx: AuditContext<'_>,
    audit: Option<Audit>,
) -> Result<(), AdminApiError> {
    let audit = audit.unwrap_or_default();
    let entry = NewConsoleAudit {
        actor: Actor::User(ctx.user_id),
        action: audit
            .action
            .map_or_else(|| ctx.operation_id.to_owned(), Cow::into_owned),
        target_type: audit.target_type.into_owned(),
        target_id: audit.target_id,
        before: audit.before,
        after: audit.after,
        ip: Some(ctx.ip),
    };
    match state.console_audit.record(&entry).await {
        Ok(_) => Ok(()),
        Err(err) => {
            let shown = |v: &Option<Value>| v.as_ref().map(|v| redact(v).to_string());
            tracing::error!(
                actor_id = ctx.user_id,
                action = %entry.action,
                target_type = %entry.target_type,
                target_id = %entry.target_id,
                before = ?shown(&entry.before),
                after = ?shown(&entry.after),
                ip = %ctx.ip,
                "console audit write failed; the change was applied but is not in console_audit"
            );
            Err(AdminApiError::internal(err))
        }
    }
}
