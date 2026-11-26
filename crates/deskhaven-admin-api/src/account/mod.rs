//! Every route acts on the session's user, never on an id from the request. The password
//! change and the TOTP confirmation change the session's standing, so both answer with a
//! fresh console session.

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
