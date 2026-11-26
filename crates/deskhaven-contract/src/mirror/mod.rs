//! What the official RustDesk client deserializes. `Deserialize` only: these types must never
//! produce a response (that would be the server confirming itself). They mimic the client's
//! tolerance, but where the client silently drops a mistyped field the mirror is stricter, and
//! where Flutter and the Rust client both read a payload it is the stricter of the two.

mod ab;
mod account;
mod device;
mod group;
mod hbb;
mod legacy_ab;

use serde::Deserialize;

pub use ab::{AbPeer, AbPersonal, AbProfile, AbRule, AbSettings, AbTag, PagedResponse};
pub use account::{
    AuthBody, ChallengeUser, ClientUserPayload, ClientUserStatus, DeviceInfo, LoginOptions,
    LoginResponse, OidcAuthUrl, SecondFactorChallenge, UserInfo, WhitelistItem,
};
pub use device::{DeployResponse, HeartbeatResponse, StrategyOptions};
pub use group::{DeviceGroupPayload, PeerInfo, PeerPayload};
pub use hbb::HbbData;
pub use legacy_ab::{LegacyAbData, LegacyAbReply};

/// `error` must be a string; it is shown to the user as is.
#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}
