//! The signed-in user's own credentials. The first three are `Session` routes (reachable
//! while the account setup is unfinished, which they finish); the rest are `self` routes
//! (M3-13, the account page). Every one acts on the session's user, never on an id from
//! the request.
//!
//! | method | path | what | audit |
//! |---|---|---|---|
//! | `POST` | `/me/password` | change the password (current one required) | `user.password_change` |
//! | `POST` | `/me/totp/enrolment` | a new pending TOTP seed (text, URI, QR) | `user.totp_enrolment_begin` |
//! | `POST` | `/me/totp/enrolment/confirm` | enable TOTP with a code; recovery codes | `user.totp_enable` |
//! | `POST` | `/me/totp/disable` | turn TOTP off (password and code; refused when enforced; ends other sessions) | `user.totp_disable` |
//! | `POST` | `/me/totp/recovery-codes` | replace the recovery codes (password and code) | `user.recovery_codes_regenerate` |
//! | `GET` | `/me/account` | the account page's data | |
//! | `PATCH` | `/me` | display name | `user.profile_update` |
//! | `POST` | `/me/identities/link` | start linking an OIDC identity (password, and a code with TOTP) | `user.identity_link_begin` |
//! | `POST` | `/me/identities/link/verify` | exchange the IdP answer once, show the IdP identity | `user.identity_link_verify` |
//! | `POST` | `/me/identities/link/complete` | link it to this account | `user.identity_link` |
//! | `POST` | `/me/identities/{id}/unlink` | remove an OIDC identity (password) | `user.identity_unlink` |
//!
//! The password change and the TOTP confirmation change the session's standing, so both
//! answer with a fresh console session (cookies): a password change revokes every session
//! of the user (06 §3.2), the TOTP confirmation rotates every console session of the user.

mod identities;
mod link;
mod link_confirm;
mod password;
mod profile;
mod qr;
mod reauth;
mod totp;
mod totp_manage;

use utoipa_axum::routes;

use crate::routes::{AdminRoutes, RouteError};

pub use identities::IdentityUnlink;
pub(crate) use identities::unlink_audit;
pub use link::OidcLinkStart;
pub use link_confirm::{OidcLinkConfirm, OidcLinkPreview};
pub use password::PasswordChange;
pub use profile::{MyAccount, ProfilePatch};
pub use qr::QrCode;
pub(crate) use qr::encode as qr_code;
pub use totp::{TotpConfirm, TotpEnabled, TotpEnrolment};
pub use totp_manage::{RecoveryCodesIssued, TotpProof};

pub(crate) fn routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new()
        .routes(routes!(password::change))?
        .routes(routes!(totp::begin))?
        .routes(routes!(totp::confirm))?
        .routes(routes!(totp_manage::disable))?
        .routes(routes!(totp_manage::regenerate))?
        .routes(routes!(profile::view))?
        .routes(routes!(profile::edit))?
        .routes(routes!(link::start))?
        .routes(routes!(link_confirm::verify))?
        .routes(routes!(link_confirm::complete))?
        .routes(routes!(identities::unlink))
}
