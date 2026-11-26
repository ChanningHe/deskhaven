use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::state::AppState;
use serde_json::Value;

use crate::{Audit, AuditCtx};

const TARGET: &str = "address_book";

pub(crate) fn book_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
    guid: Option<&str>,
) -> TxGuard<Value> {
    let entry_ctx = ctx.clone();
    let guid = guid.map(str::to_owned);
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Value>, after: Option<&Value>| {
            let target = guid.unwrap_or_else(|| guid_of(after.or(before)));
            let mut audit = Audit::new(action).target(TARGET, target);
            if let Some(before) = before {
                audit = audit.before(before);
            }
            if let Some(after) = after {
                audit = audit.after(after);
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

fn guid_of(snapshot: Option<&Value>) -> String {
    let guid = snapshot.and_then(|s| s.pointer("/book/guid"));
    guid.and_then(Value::as_str).unwrap_or_default().to_owned()
}
