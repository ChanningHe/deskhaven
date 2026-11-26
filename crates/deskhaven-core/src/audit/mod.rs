//! The client retries a record for up to 120 s with the same `nonce`; the nonce is stored in
//! the record's own transaction, so a repeat writes nothing and a failed write claims nothing.
//! Self-reported text is cleaned and cut, never refused: a refusal loses the record for good.

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub rustdesk_id: String,
    pub uuid: String,
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Conn(ConnReport),
    File(FileReport),
    Alarm(AlarmReport),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingest {
    Stored,
    Duplicate,
    NotFound,
    /// The device is over [`AUDIT_PER_DEVICE_PER_MINUTE`]: nothing written, try again later.
    TooFrequent {
        retry_after: Duration,
    },
}

/// `id` is the remote, i.e. the controlled, device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerNote {
    pub rustdesk_id: String,
    pub session_id: SessionId,
    pub note: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteOutcome {
    Stored,
    /// Empty once cleaned, or the session has its note already (the first stays).
    Ignored,
    Pending,
    Full,
    NotFound,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuditError {
    /// Whether it committed is unknown; a retry with the same nonce settles it.
    #[error("audit write did not complete")]
    Aborted,

    #[error("device verification failed")]
    Device(#[from] DeviceError),

    #[error("audit storage failed")]
    Database(#[from] DbErr),
}
