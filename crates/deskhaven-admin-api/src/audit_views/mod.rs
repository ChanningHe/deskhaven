//! Audit views (M3-08): read-only lists of the connection, file transfer, alarm, login and
//! console audit records. Admin routes; a list writes nothing (reading the audit trail is
//! not itself audited: it changes nothing, and a row per page view would bury the trail).
//!
//! A plain user cannot read their own login events here: that belongs to the "my account"
//! pages (M3-13), which scope by the session, not by a filter the caller controls.
//!
//! Every device-reported text (peer names, paths, notes, alarm info) is returned as stored
//! and rendered as text by the console; see 06 §2.

mod alarms;
mod connections;
mod console;
mod files;
mod filter;
mod logins;
mod view;

use utoipa_axum::routes;

pub(crate) use alarms::AlarmAuditView;

use crate::{AdminRoutes, RouteError};

/// The `/audit/*` routes.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(connections::list_connection_audit))?
        .routes(routes!(files::list_file_audit))?
        .routes(routes!(alarms::list_alarm_audit))?
        .routes(routes!(logins::list_login_audit))?
        .routes(routes!(console::list_console_audit))
}
