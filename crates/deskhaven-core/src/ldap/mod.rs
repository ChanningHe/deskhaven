//! Directory accounts never fall back to a local password, and an unreachable directory fails
//! the login. `test_user` with a password is a password oracle: its API is throttled, counted
//! against the username's lockout and limited to the owner. Sessions are not tied to the
//! directory: a change there is seen at the next login. No password ever reaches an error, a log
//! line or a diagnostic.

mod account;
mod address;
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
mod report;
mod settings;
mod sync;
mod tls;
mod validate;
mod verify;

use std::sync::Arc;

use sea_orm::DatabaseConnection;

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
pub use report::{Diagnostic, ProbedUser, Step, StepReport, StepStatus};
pub use settings::DirectorySettings;
pub use verify::DirectoryLimits;

use crate::clock::SharedClock;
use crate::console_audit::ConsoleAuditLog;
use crate::crypto::SecretBox;
use crate::groups::GroupService;
use crate::users::UserService;

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

    pub fn with_limits(self, limits: DirectoryLimits) -> Self {
        Self {
            gate: Arc::new(verify::Gate::new(limits)),
            ..self
        }
    }
}
