//! Wire types of the user endpoints. Timestamps are unix milliseconds (UTC).

use deskhaven_core::groups::Group;
use deskhaven_core::sessions::{Session, SessionKind};
use deskhaven_core::users::{AuthSource, Password, Status, User};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::RoleView;

/// Account status (04 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusView {
    Active,
    Disabled,
    Unverified,
}

impl From<Status> for StatusView {
    fn from(status: Status) -> Self {
        match status {
            Status::Active => Self::Active,
            Status::Disabled => Self::Disabled,
            Status::Unverified => Self::Unverified,
        }
    }
}

impl From<StatusView> for Status {
    fn from(status: StatusView) -> Self {
        match status {
            StatusView::Active => Self::Active,
            StatusView::Disabled => Self::Disabled,
            StatusView::Unverified => Self::Unverified,
        }
    }
}

/// Where the account authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthSourceView {
    Local,
    Ldap,
    Oidc,
}

impl From<AuthSource> for AuthSourceView {
    fn from(source: AuthSource) -> Self {
        match source {
            AuthSource::Local => Self::Local,
            AuthSource::Ldap => Self::Ldap,
            AuthSource::Oidc => Self::Oidc,
        }
    }
}

impl From<AuthSourceView> for AuthSource {
    fn from(source: AuthSourceView) -> Self {
        match source {
            AuthSourceView::Local => Self::Local,
            AuthSourceView::Ldap => Self::Ldap,
            AuthSourceView::Oidc => Self::Oidc,
        }
    }
}

/// A user account as administrators see it. Carries no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct UserView {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub auth_source: AuthSourceView,
    pub role: RoleView,
    pub status: StatusView,
    pub must_change_password: bool,
    pub totp_enabled: bool,
    pub note: String,
    pub last_login_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<User> for UserView {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            email: u.email,
            email_verified: u.email_verified,
            auth_source: u.auth_source.into(),
            role: u.role.into(),
            status: u.status.into(),
            must_change_password: u.must_change_password,
            totp_enabled: u.totp_enabled,
            note: u.note,
            last_login_at: u.last_login_at.map(|t| t.timestamp_millis()),
            created_at: u.created_at.timestamp_millis(),
            updated_at: u.updated_at.timestamp_millis(),
        }
    }
}

/// A group a user belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct GroupRef {
    pub id: i64,
    pub name: String,
}

impl From<Group> for GroupRef {
    fn from(g: Group) -> Self {
        Self {
            id: g.id,
            name: g.name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionKindView {
    Client,
    Console,
}

/// A usable session of the user (no token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionView {
    pub id: i64,
    pub kind: SessionKindView,
    /// Client sessions: the RustDesk id of the device.
    pub rustdesk_id: Option<String>,
    pub ip: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
    pub expires_at: i64,
}

impl From<Session> for SessionView {
    fn from(s: Session) -> Self {
        Self {
            id: s.id,
            kind: match s.kind {
                SessionKind::Client => SessionKindView::Client,
                SessionKind::Console => SessionKindView::Console,
            },
            rustdesk_id: s.device.map(|d| d.rustdesk_id),
            ip: s.ip.map(|ip| ip.to_string()),
            created_at: s.created_at.timestamp_millis(),
            last_used_at: s.last_used_at.timestamp_millis(),
            expires_at: s.expires_at.timestamp_millis(),
        }
    }
}

/// One account with its groups and usable sessions (the console's user drawer).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct UserDetail {
    pub user: UserView,
    pub groups: Vec<GroupRef>,
    /// The most recently used usable sessions, at most 200 (`sessions_total` counts all).
    pub sessions: Vec<SessionView>,
    /// Usable sessions of the account.
    pub sessions_total: u64,
}

/// A secret a response shows once (a temporary password): serialized as the plain string,
/// but `Debug` prints `<redacted>`, so a logged response or error never carries it.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ShownSecret(String);

impl ShownSecret {
    pub fn new(password: &Password) -> Self {
        Self(password.expose().to_owned())
    }
}

impl std::fmt::Debug for ShownSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A created account; `temporary_password` only when the server generated one (shown once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct UserCreated {
    pub user: UserView,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub temporary_password: Option<ShownSecret>,
}

/// A generated temporary password, shown once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TemporaryPassword {
    #[schema(value_type = String)]
    pub temporary_password: ShownSecret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct SessionsRevoked {
    /// Sessions that were still usable.
    pub revoked: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
pub struct LockoutCleared {
    /// Lockout counters removed (0: nothing was locked or counted).
    pub cleared: u64,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn shown_secrets_serialize_plainly_but_never_debug_print() {
        let secret = ShownSecret::new(&Password::new("Tmp-Secret-123"));
        let reset = TemporaryPassword {
            temporary_password: secret.clone(),
        };
        let json = serde_json::to_value(&reset).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "temporary_password": "Tmp-Secret-123" })
        );
        assert!(!format!("{reset:?}").contains("Tmp-Secret"));
        assert_eq!(format!("{:?}", Some(secret)), "Some(<redacted>)");
    }
}
