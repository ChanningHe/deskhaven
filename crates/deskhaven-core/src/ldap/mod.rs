//! LDAP directory logins (06 §3.5).
//!
//! - **Directories** (`directories`, `model`): P0 enables one. Rows of `ldap_directories`,
//!   the bind password sealed with [`SecretBox`] (06 §5). TLS by default: `ldaps://` or
//!   StartTLS, certificates verified unless `tls_verify` is turned off explicitly; plain
//!   `ldap://` only to a loopback host.
//! - **Logins** (`login`): [`LdapService::authenticate_password`] routes a password login
//!   to the local password or the directory by the account's `auth_source`; directory
//!   accounts never fall back to a local password, and an unreachable directory fails the
//!   login. Service bind -> search with every input escaped (`filter`) -> user bind.
//! - **Accounts** (`account`, `mapping`): provisioned at the first login, role (admin group)
//!   and group memberships synchronized at every login.
//! - **Diagnostics** (`diagnose`): "test connection" / "test user" with a result per step.
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
mod settings;
mod tls;
mod validate;

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
pub use model::{
    DEFAULT_USER_FILTER, DEFAULT_USERNAME_ATTR, Directory, DirectoryUpdate, NewDirectory,
};
pub use settings::DirectorySettings;

use crate::clock::SharedClock;
use crate::crypto::SecretBox;
use crate::groups::GroupService;
use crate::users::UserService;

/// Directories and directory logins. Cheap to clone (shared state).
#[derive(Clone)]
pub struct LdapService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    users: UserService,
    /// The shared instance: membership changes reach its commit hooks (strategy cache).
    groups: GroupService,
    clock: SharedClock,
    connector: Arc<dyn Connector>,
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
            db,
            crypto,
            users,
            groups,
            clock,
            connector,
        };
        Self {
            inner: Arc::new(inner),
        }
    }
}
