use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

/// `NotFound` also covers a book the actor may not see, so a caller learns nothing about
/// books outside its reach.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AddressBookError {
    #[error("address book not found")]
    NotFound,

    #[error("permission denied")]
    Forbidden,

    #[error("user not found")]
    UserNotFound,

    #[error("peer not found")]
    PeerNotFound,

    #[error("peer already exists")]
    PeerExists,

    #[error("tag not found")]
    TagNotFound,

    #[error("tag already exists")]
    TagExists,

    #[error("the address book is full ({max} peers)")]
    Full { max: u32 },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("address book name is already taken")]
    NameTaken,

    #[error("several address books have this name")]
    AmbiguousName,

    #[error("share subject not found")]
    SubjectNotFound,

    #[error("personal address books cannot be shared or managed")]
    Personal,

    #[error(transparent)]
    Refused(#[from] Refused),

    #[error("address book secret could not be sealed")]
    Crypto(#[source] CryptoError),

    #[error("address book storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for AddressBookError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for AddressBookError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> AddressBookError {
    AddressBookError::Invalid {
        field,
        reason: reason.into(),
    }
}
