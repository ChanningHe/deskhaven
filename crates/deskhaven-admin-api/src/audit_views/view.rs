//! Wire pieces shared by the audit lists. Timestamps are unix milliseconds (UTC).

use deskhaven_core::audit_query::AuditUser;
use serde::Serialize;
use utoipa::ToSchema;

/// A user account an audit record points at. Absent in the record when the account no
/// longer exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AuditUserView {
    pub id: i64,
    pub username: String,
}

impl From<AuditUser> for AuditUserView {
    fn from(user: AuditUser) -> Self {
        Self {
            id: user.id,
            username: user.username,
        }
    }
}
