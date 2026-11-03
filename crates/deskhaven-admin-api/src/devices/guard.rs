//! Audit guard of device writes: any admin may manage devices, so beyond re-checking that
//! the actor is still an active admin or owner ([`AuditCtx::admin_guard`]) it never refuses.

use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::devices::DeviceRecord;
use deskhaven_core::state::AppState;
use serde::Serialize;
use serde_json::json;

use super::view::DeviceStatusView;
use crate::{Audit, AuditCtx};

/// Snapshot of a device in its audit rows: what the console can change, and the id.
#[derive(Serialize)]
struct DeviceSnapshot<'a> {
    rustdesk_id: &'a str,
    owner_user_id: Option<i64>,
    device_group_id: Option<i64>,
    note: &'a str,
    status: DeviceStatusView,
}

impl<'a> From<&'a DeviceRecord> for DeviceSnapshot<'a> {
    fn from(d: &'a DeviceRecord) -> Self {
        Self {
            rustdesk_id: &d.rustdesk_id,
            owner_user_id: d.owner_user_id,
            device_group_id: d.device_group_id,
            note: d.note.as_deref().unwrap_or_default(),
            status: d.status.into(),
        }
    }
}

/// The guard of a disconnect request: the actor is re-checked in the transaction and the row
/// names the connection (`after`: `{rustdesk_id, conn_id}`).
pub(super) fn disconnect_guard(
    state: &AppState,
    ctx: &AuditCtx,
    device_id: i64,
    conn_id: i32,
) -> TxGuard<DeviceRecord> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, _before: Option<&DeviceRecord>, after: Option<&DeviceRecord>| {
            let rustdesk_id = after.map_or("", |d| d.rustdesk_id.as_str());
            let snapshot = json!({ "rustdesk_id": rustdesk_id, "conn_id": conn_id });
            let audit = Audit::new("device.disconnect")
                .target("device", device_id)
                .after(&snapshot);
            Ok(entry_ctx.entry(audit))
        },
    )
}

pub(super) fn device_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
    device_id: i64,
) -> TxGuard<DeviceRecord> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&DeviceRecord>, after: Option<&DeviceRecord>| {
            let mut audit = Audit::new(action).target("device", device_id);
            if let Some(before) = before {
                audit = audit.before(&DeviceSnapshot::from(before));
            }
            if let Some(after) = after {
                audit = audit.after(&DeviceSnapshot::from(after));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}
