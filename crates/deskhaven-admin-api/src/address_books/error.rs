//! Address book service errors -> enveloped admin API errors. Messages are fixed English
//! text; `details` carries only field names and policy reasons, never input values.

use deskhaven_core::addressbooks::AddressBookError;
use serde_json::json;

use crate::AdminApiError;
use crate::domain_error::{invalid_field, refused};

impl From<AddressBookError> for AdminApiError {
    fn from(err: AddressBookError) -> Self {
        use AddressBookError as E;
        match err {
            // A personal book (never reachable here) is "not found" like a missing one.
            E::NotFound | E::Personal => Self::not_found("address book not found"),
            E::Forbidden => Self::forbidden(),
            E::UserNotFound => Self::not_found("user not found"),
            E::SubjectNotFound => Self::not_found("share subject not found"),
            E::PeerNotFound => Self::not_found("entry not found"),
            E::TagNotFound => Self::not_found("tag not found"),
            E::PeerExists => conflict("entry already exists", json!({ "field": "id" })),
            E::TagExists => conflict("tag already exists", json!({ "field": "name" })),
            E::NameTaken => conflict(
                "address book name is already taken",
                json!({ "field": "name" }),
            ),
            E::AmbiguousName => conflict(
                "several address books have this name",
                json!({ "field": "name" }),
            ),
            E::Full { max } => conflict(
                "the address book is full",
                json!({ "reason": "book_full", "max": max }),
            ),
            E::Invalid { field, reason } => invalid_field(field, reason),
            E::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}

fn conflict(message: &'static str, details: serde_json::Value) -> AdminApiError {
    AdminApiError::conflict(message).with_details(details)
}
