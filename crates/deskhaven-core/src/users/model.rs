//! Domain types of the user service.

use deskhaven_db::entity::user::{self, AuthSource, Role, Status};
use deskhaven_db::time::TimestampMs;

use super::Password;
use crate::pagination::PageRequest;

/// A user account as seen outside the service. Carries no secret (no password hash, no TOTP
/// seed), so it is safe to hand to API layers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub auth_source: AuthSource,
    pub role: Role,
    pub status: Status,
    pub must_change_password: bool,
    pub totp_enabled: bool,
    pub note: String,
    pub avatar_path: Option<String>,
    pub last_login_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<user::Model> for User {
    fn from(m: user::Model) -> Self {
        Self {
            id: m.id,
            username: m.username,
            display_name: m.display_name,
            email: m.email,
            email_verified: m.email_verified,
            auth_source: m.auth_source,
            role: m.role,
            status: m.status,
            must_change_password: m.must_change_password,
            totp_enabled: m.totp_enabled_at.is_some(),
            note: m.note,
            avatar_path: m.avatar_path,
            last_login_at: m.last_login_at,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// How a new account authenticates; decides `auth_source`.
#[derive(Clone, Debug)]
pub enum Credential {
    /// Local password (checked against the policy, stored as an Argon2id hash).
    Local { password: Password },
    /// Directory account: no local password, every login goes to LDAP.
    Ldap,
    /// OIDC account: no local password.
    Oidc,
}

impl Credential {
    pub(super) fn auth_source(&self) -> AuthSource {
        match self {
            Self::Local { .. } => AuthSource::Local,
            Self::Ldap => AuthSource::Ldap,
            Self::Oidc => AuthSource::Oidc,
        }
    }
}

/// Input of [`UserService::create`](super::UserService::create).
#[derive(Clone, Debug)]
pub struct NewUser {
    pub username: String,
    /// Blank: the username.
    pub display_name: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub role: Role,
    pub status: Status,
    pub must_change_password: bool,
    pub note: String,
    pub credential: Credential,
}

impl NewUser {
    /// An active `user`-role account with no email, note or display name.
    pub fn new(username: impl Into<String>, credential: Credential) -> Self {
        Self {
            username: username.into(),
            display_name: String::new(),
            email: None,
            email_verified: false,
            role: Role::User,
            status: Status::Active,
            must_change_password: false,
            note: String::new(),
            credential,
        }
    }
}

/// Profile fields to change; `None` leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct ProfileUpdate {
    /// Blank: the username.
    pub display_name: Option<String>,
    /// `Some(None)` removes the address. A different address (case-insensitively) resets
    /// `email_verified` unless it is set explicitly in the same update.
    pub email: Option<Option<String>>,
    pub email_verified: Option<bool>,
    pub must_change_password: Option<bool>,
    pub note: Option<String>,
}

/// Filters of [`UserService::list`](super::UserService::list). `q` matches a substring of
/// the username, email or display name, case-insensitively and literally (no wildcards).
/// Results are ordered by `sort` (if any), then by id (in the same direction).
#[derive(Clone, Debug, Default)]
pub struct UserQuery {
    pub status: Option<Status>,
    pub role: Option<Role>,
    pub auth_source: Option<AuthSource>,
    /// Members of this group only.
    pub group_id: Option<i64>,
    /// Accounts that are not members of this group (candidates to add).
    pub not_group_id: Option<i64>,
    pub q: Option<String>,
    pub sort: Option<UserSort>,
    pub page: PageRequest,
}

/// Columns a user list can be ordered by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserSortField {
    Username,
    DisplayName,
    Role,
    Status,
    CreatedAt,
    LastLoginAt,
}

impl UserSortField {
    pub(super) fn column(self) -> user::Column {
        match self {
            Self::Username => user::Column::UsernameNorm,
            Self::DisplayName => user::Column::DisplayName,
            Self::Role => user::Column::Role,
            Self::Status => user::Column::Status,
            Self::CreatedAt => user::Column::CreatedAt,
            Self::LastLoginAt => user::Column::LastLoginAt,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserSort {
    pub field: UserSortField,
    pub descending: bool,
}
