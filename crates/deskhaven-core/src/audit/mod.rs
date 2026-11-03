//! Audit ingest (03 §2.3-§2.6, 04 §7, 06 §2 / §7): the records the client posts to
//! `/api/audit/{conn,file,alarm}`.
//!
//! - **Identity**: records of the controlled device carry `id` + `uuid` and are refused
//!   ([`Ingest::NotFound`], nothing written) unless
//!   [`DeviceService::verify`](crate::devices::DeviceService::verify) accepts them (unknown id
//!   and uuid mismatch alike). The controller's session note carries no uuid; its id must be
//!   registered (cached lookup), and the HTTP layer answers it the same either way.
//! - **Rate limits** (see `limits`): per client network on every request, per device after
//!   verification ([`Ingest::TooFrequent`]).
//! - **De-duplication**: the client retries a record for up to 120 s with the same `nonce`
//!   (`src/server/connection.rs:1674-1760`). The nonce goes into `audit_nonces` in the
//!   record's own transaction: a repeat is [`Ingest::Duplicate`] (answered like success,
//!   nothing written), a failed write claims nothing. `received_at` lets the retention job
//!   prune nonces (kept ≥ 24 h, far past the client's window). Records without a nonce (older
//!   clients, which never retry) are stored as they come.
//! - **Merging**: the three records of a connection become one `connection_sessions` row
//!   in any order (see `conn`); the controller's first non-empty note joins the session's
//!   rows, waiting in a bounded buffer if it comes first (see `notes`).
//! - **Self-reported text** (peer id and name, paths, file names, notes, alarm `info` keys
//!   and strings) is cleaned (control and invisible characters dropped) and cut to a length,
//!   never refused: a refusal loses the record for good. Alarm `info` is stored flat.

mod conn;
mod limits;
mod notes;
mod records;
mod service;
mod session_id;

pub use conn::{Authorization, ConnEvent, ConnReport, PAIRING_WINDOW, RETIRED_PREFIX};
pub use limits::{AUDIT_PER_DEVICE_PER_MINUTE, AUDIT_PER_IP_PER_MINUTE};
pub use notes::{MAX_NOTES_PER_SOURCE, MAX_NOTES_PER_SOURCE_DEVICE, MAX_PENDING_NOTES, NOTE_TTL};
pub use records::{
    AlarmReport, Direction, FileInfo, FileReport, MAX_FILES, MAX_INFO_KEYS, MAX_INFO_TEXT_CHARS,
};
pub use service::AuditService;
pub use session_id::SessionId;

use std::time::Duration;

use sea_orm::DbErr;

use crate::devices::DeviceError;

/// Who reports a record: the device's RustDesk id and uuid (base64, as sent), and the
/// record's nonce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub rustdesk_id: String,
    pub uuid: String,
    pub nonce: Option<String>,
}

/// A record of the controlled device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Conn(ConnReport),
    File(FileReport),
    Alarm(AlarmReport),
}

/// Result of [`AuditService::ingest`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingest {
    Stored,
    /// The nonce was stored before: nothing written.
    Duplicate,
    /// Unknown id, or a uuid that does not match: nothing written.
    NotFound,
    /// The device is over [`AUDIT_PER_DEVICE_PER_MINUTE`]: nothing written, try again later.
    TooFrequent {
        retry_after: Duration,
    },
}

/// The controller's note of a session (`src/ui_session_interface.rs:592-598`, `2104-2107`):
/// `id` is the remote, i.e. the controlled, device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerNote {
    /// The controlled device.
    pub rustdesk_id: String,
    pub session_id: SessionId,
    pub note: String,
}

/// Result of [`AuditService::record_note`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteOutcome {
    /// Set on the session's rows.
    Stored,
    /// Nothing to do: the note is empty once cleaned, or the session has its note already
    /// (the first one stays, 06 §7).
    Ignored,
    /// No row has the session yet: kept until one does, at most [`NOTE_TTL`].
    Pending,
    /// No row has the session yet, and no room to wait: the source, the device or the whole
    /// buffer is at its cap ([`MAX_NOTES_PER_SOURCE`], [`MAX_NOTES_PER_SOURCE_DEVICE`],
    /// [`MAX_PENDING_NOTES`]).
    Full,
    /// The id is not registered.
    NotFound,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuditError {
    /// The write's task panicked or the runtime is shutting down; whether it committed is
    /// unknown (a retry with the same nonce settles it).
    #[error("audit write did not complete")]
    Aborted,

    #[error("device verification failed")]
    Device(#[from] DeviceError),

    #[error("audit storage failed")]
    Database(#[from] DbErr),
}
