//! LDAP directory logins (06 §3.5).
//!
//! - **Directories** (`directories`, `model`): P0 enables one. Rows of `ldap_directories`,
//!   the bind password sealed with [`SecretBox`] (06 §5). TLS by default: `ldaps://` or
//!   StartTLS, certificates verified unless `tls_verify` is turned off explicitly; plain
//!   `ldap://` only to a loopback host; no link-local or cloud metadata address literals
//!   (host names are resolved by ldap3 and not filtered). Changing where the bind password
//!   goes (URL, TLS settings, bind DN) needs it entered again.
//! - **Logins** (`login`, `verify`): [`LdapService::password_login`] routes a password
//!   login to the local password or the directory by the account's `auth_source`;
//!   directory accounts never fall back to a local password, and an unreachable directory
//!   fails the login. Service bind -> search with every input escaped (`filter`) -> user
//!   bind, bounded in concurrency and total time. TOTP applies to directory accounts like
//!   to local ones (the second step is the login endpoints'); `reauth` re-checks a
//!   directory password for self-service changes.
//! - **Accounts** (`account`, `mapping`, `sync`): provisioned at the first login, role (admin group)
//!   and group memberships synchronized at every login, audited as the system.
//! - **Diagnostics** (`diagnose`): "test connection" / "test user" with a result per step.
//!   `test_user` with a password is a password oracle: its API must be throttled and
//!   limited to administrators (M3-09).
//!
//! Sessions are not tied to the directory: an account disabled or a password changed in
//! the directory keeps its existing deskhaven sessions until they expire or are revoked;
//! the next login sees the change. There is no custom CA setting yet (the OS roots verify).
//!
//! The protocol sits behind [`Connector`] / [`Session`]; production uses
//! [`Ldap3Connector`]. No password ever reaches an error, a log line or a diagnostic.

mod account;
mod diagnose;
mod directories;
mod directory;
mod error;
mod filter;
mod ldap3_conn;
mod login;
mod mapping;
mod model;
mod protocol;
mod reauth;
mod settings;
mod sync;
mod tls;
mod validate;
mod verify;

use std::sync::Arc;

use sea_orm::DatabaseConnection;

pub use diagnose::{Diagnostic, ProbedUser, Step, StepReport, StepStatus};
pub use directory::{
    Connector, DirectoryFault, DirectoryFuture, Entry, Found, Scope, SearchRequest, Session, Target,
};
pub use error::DirectoryError;
pub use filter::{
    DEFAULT_GROUP_FILTER, USER_DN_PLACEHOLDER, USERNAME_PLACEHOLDER, escape_filter_value,
};
pub use ldap3_conn::{CONNECT_TIMEOUT, Ldap3Connector, OPERATION_TIMEOUT};
pub use login::PasswordLogin;
pub use model::{
    DEFAULT_USER_FILTER, DEFAULT_USERNAME_ATTR, Directory, DirectoryUpdate, NewDirectory,
};
pub use settings::DirectorySettings;
pub use verify::DirectoryLimits;

use crate::clock::SharedClock;
use crate::console_audit::ConsoleAuditLog;
use crate::crypto::SecretBox;
use crate::groups::GroupService;
use crate::users::UserService;

/// Directories and directory logins. Cheap to clone (shared state).
#[derive(Clone)]
pub struct LdapService {
    inner: Arc<Inner>,
    gate: Arc<verify::Gate>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    users: UserService,
    /// The shared instance: membership changes reach its commit hooks (strategy cache).
    groups: GroupService,
    clock: SharedClock,
    connector: Arc<dyn Connector>,
    /// Role and membership changes the directory makes (actor `system`).
    audit: ConsoleAuditLog,
}

impl LdapService {
    pub fn new(
        db: DatabaseConnection,
        crypto: Arc<SecretBox>,
        users: UserService,
        groups: GroupService,
        clock: SharedClock,
        connector: Arc<dyn Connector>,
    ) -> Self {
        let inner = Inner {
            audit: ConsoleAuditLog::new(db.clone(), Arc::clone(&clock)),
            db,
            crypto,
            users,
            groups,
            clock,
            connector,
        };
        Self {
            inner: Arc::new(inner),
            gate: Arc::new(verify::Gate::new(DirectoryLimits::default())),
        }
    }

    /// The same service with other bounds on directory work (fresh slots).
    pub fn with_limits(self, limits: DirectoryLimits) -> Self {
        Self {
            gate: Arc::new(verify::Gate::new(limits)),
            ..self
        }
    }
}
