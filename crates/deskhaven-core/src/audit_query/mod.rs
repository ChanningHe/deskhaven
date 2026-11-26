//! Self-reported data is returned as stored; showing it as untrusted text is the caller's job.
//! Connection and alarm `ip` columns hold what the device claims, not what the server saw.

mod activity;
mod alarms;
mod connections;
mod console;
mod files;
mod listing;
mod logins;
mod names;
mod stored;

use sea_orm::{DatabaseConnection, DbErr};

pub use activity::Activity;
pub use alarms::{AlarmQuery, alarm_info};
pub use connections::{ConnectionEntry, ConnectionQuery};
pub use console::{ConsoleActor, ConsoleEntry, ConsoleQuery, UNREADABLE_SNAPSHOT, parse_snapshot};
pub use deskhaven_db::entity::alarm::Model as AlarmRow;
pub use deskhaven_db::entity::console_audit::ActorKind;
pub use deskhaven_db::entity::file_transfer::Direction as TransferDirection;
pub use deskhaven_db::entity::file_transfer::Model as FileRow;
pub use files::{FileQuery, TransferredFile, transferred_files};
pub use listing::Listing;
pub use logins::{LoginEntry, LoginQuery};
pub use names::AuditUser;
pub use stored::{parse_stored, stored_name};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuditQueryError {
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("audit query failed")]
    Database(#[from] DbErr),
}

#[derive(Clone)]
pub struct AuditQuery {
    db: DatabaseConnection,
}

impl AuditQuery {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}
