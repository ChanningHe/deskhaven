//! Client-mirror types: what the official RustDesk client deserializes, at the
//! commit in `tests/contract/UPSTREAM_COMMIT`. Paths in the doc comments are
//! relative to the rustdesk repository.
//!
//! Rules:
//! - `Deserialize` only. These types describe what the *client* reads; they
//!   must never be used to produce a response (that would be the server
//!   confirming itself), and production crates cannot reach them anyway.
//! - Mimic the client's tolerance: unknown fields are ignored, fields the
//!   client defaults (`?? ''`, `#[serde(default)]`) are optional, and a value
//!   of the wrong JSON type fails (Dart throws a `TypeError` assigning it,
//!   serde rejects it).
//! - Where the client is more lenient than correct (it silently ignores a
//!   mistyped field or reads it as `false`), the mirror is stricter and says
//!   so: a field the client drops is a contract bug.
//! - When the Flutter and the Rust client both read a payload, the mirror is
//!   the stricter of the two.

mod ab;
mod account;
mod device;
mod hbb;

use serde::Deserialize;

pub use ab::{AbPeer, PagedResponse};
pub use account::{
    AuthBody, ChallengeUser, ClientUserPayload, ClientUserStatus, DeviceInfo, LoginOptions,
    LoginResponse, OidcAuthUrl, SecondFactorChallenge, UserInfo, WhitelistItem,
};
pub use device::{HeartbeatResponse, StrategyOptions};
pub use hbb::HbbData;

/// Error body of every client API endpoint (03 §1.3).
///
/// - `src/hbbs_http.rs:24-39`: `error` must be a string (`null` included in
///   the failures), else `ErrorFormat`.
/// - `flutter/lib/models/user_model.dart:209-214` (`RequestException` takes a
///   `String`), `flutter/lib/models/ab_model.dart:1457-1461`: `body['error']`
///   is shown to the user as is.
#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn error_body_requires_a_string() {
        assert!(serde_json::from_value::<ErrorBody>(json!({ "error": "denied" })).is_ok());
        assert!(serde_json::from_value::<ErrorBody>(json!({ "error": 1 })).is_err());
        assert!(serde_json::from_value::<ErrorBody>(json!({ "message": "x" })).is_err());
    }
}
