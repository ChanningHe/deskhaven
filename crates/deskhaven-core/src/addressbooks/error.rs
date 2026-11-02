use sea_orm::DbErr;

use crate::crypto::CryptoError;

/// Why an address book operation failed. Variants are stable for API error mapping.
///
/// [`NotFound`](Self::NotFound) covers both "no such book" and "a book the actor may not
/// see", so a caller learns nothing about books outside its reach.
/// [`Forbidden`](Self::Forbidden) is only returned for a book the actor can read.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AddressBookError {
    #[error("address book not found")]
    NotFound,

    /// The book is visible but the actor's rule does not allow the operation.
    #[error("permission denied")]
    Forbidden,

    /// The acting user no longer exists.
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

    /// Adding a peer would exceed `addressbook.max_peers`.
    #[error("the address book is full ({max} peers)")]
    Full { max: u32 },

    /// Malformed input; `field` names the offending input field (wire name).
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// Another shared book already has this name (compared case-insensitively), or the name
    /// is one the client reserves.
    #[error("address book name is already taken")]
    NameTaken,

    /// Several shared books carry the requested name; resolving by name refuses to guess.
    #[error("several address books have this name")]
    AmbiguousName,

    /// The user or group a share names does not exist.
    #[error("share subject not found")]
    SubjectNotFound,

    /// Personal books are never shared, renamed or deleted through the shared-book API.
    #[error("personal address books cannot be shared or managed")]
    Personal,

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

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> AddressBookError {
    AddressBookError::Invalid {
        field,
        reason: reason.into(),
    }
}
