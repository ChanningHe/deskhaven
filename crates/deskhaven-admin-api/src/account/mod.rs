//! The signed-in user's own credentials (`Session` permission: reachable while the
//! account setup is unfinished, which these routes finish; M3-13 reuses them):
//!
//! | method | path | what | audit |
//! |---|---|---|---|
//! | `POST` | `/me/password` | change the password (current one required) | `user.password_change` |
//! | `POST` | `/me/totp/enrolment` | a new pending TOTP seed (text, URI, QR) | `user.totp_enrolment_begin` |
//! | `POST` | `/me/totp/enrolment/confirm` | enable TOTP with a code; recovery codes | `user.totp_enable` |
//!
//! Both finishing steps change the session's standing, so both answer with a fresh
//! console session (cookies): a password change revokes every session of the user (06
//! §3.2), the TOTP confirmation rotates every console session of the user.

mod password;
mod qr;
mod reauth;
mod totp;

use utoipa_axum::routes;

use crate::routes::{AdminRoutes, RouteError};

pub use password::PasswordChange;
pub use qr::QrCode;
pub(crate) use qr::encode as qr_code;
pub use totp::{TotpConfirm, TotpEnabled, TotpEnrolment};

pub(crate) fn routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new()
        .routes(routes!(password::change))?
        .routes(routes!(totp::begin))?
        .routes(routes!(totp::confirm))
}
