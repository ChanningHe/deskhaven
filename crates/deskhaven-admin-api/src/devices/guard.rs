//! Audit guard of device writes: any admin may manage devices, so beyond re-checking that
//! the actor is still an active admin or owner ([`AuditCtx::admin_guard`]) it never refuses.

use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::devices::DeviceRecord;
use deskhaven_core::state::AppState;
use serde::Serialize;

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
