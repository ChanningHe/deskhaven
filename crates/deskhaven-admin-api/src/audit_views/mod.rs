//! Reading the audit trail is not itself audited: a row per page view would bury the trail.

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

pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(connections::list_connection_audit))?
        .routes(routes!(files::list_file_audit))?
        .routes(routes!(alarms::list_alarm_audit))?
        .routes(routes!(logins::list_login_audit))?
        .routes(routes!(console::list_console_audit))
}
