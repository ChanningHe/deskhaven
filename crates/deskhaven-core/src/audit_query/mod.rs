//! Reading the audit trail (M3-08, 04 §7): connection sessions, file transfers, alarms,
//! login events and the console audit log, one filtered, ordered, paginated list each.
//!
//! - **Read only.** Nothing here writes; ingest is [`crate::audit`], the console audit log
//!   [`crate::console_audit`], retention [`crate::jobs`].
//! - **Order**: by `created_at`, newest first unless asked otherwise, then by `id` the same
//!   way. Every list is served by an index on `created_at` (or a composite starting with the
//!   filtered column and ending with it), so a page costs `page_size` rows, not a sort.
//! - **Time window**: `from` inclusive, `to` exclusive, unix milliseconds.
//! - **Text** (`q`): a case-insensitive (ASCII on SQLite) literal substring of a few text
//!   columns per list; a scan, so it is the slowest filter.
//! - **Self-reported data** (peer names, paths, notes, alarm info) is returned as stored
//!   (cleaned at ingest); showing it as untrusted text is the caller's job.
//! - Rows keep their raw stored shape; the interpretation of derived values (a retired
//!   session id, a malformed file list) is on the entry types.

mod alarms;
mod connections;
mod console;
mod files;
mod listing;
mod logins;
mod names;
mod stored;

use sea_orm::{DatabaseConnection, DbErr};

pub use alarms::{AlarmQuery, alarm_info};
pub use connections::{ConnectionEntry, ConnectionQuery};
pub use console::{ConsoleActor, ConsoleEntry, ConsoleQuery, parse_snapshot};
pub use deskhaven_db::entity::alarm::Model as AlarmRow;
pub use deskhaven_db::entity::console_audit::ActorKind;
pub use deskhaven_db::entity::file_transfer::Direction as TransferDirection;
pub use deskhaven_db::entity::file_transfer::Model as FileRow;
pub use files::{FileQuery, TransferredFile, transferred_files};
pub use listing::Listing;
pub use logins::{LoginEntry, LoginQuery};
pub use names::AuditUser;
pub use stored::{parse_stored, stored_name};

/// Why an audit list could not be served.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuditQueryError {
    /// The request is out of range; `field` names the offending field.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("audit query failed")]
    Database(#[from] DbErr),
}

/// The audit lists. Cheap to clone.
#[derive(Clone)]
pub struct AuditQuery {
    db: DatabaseConnection,
}

impl AuditQuery {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}
