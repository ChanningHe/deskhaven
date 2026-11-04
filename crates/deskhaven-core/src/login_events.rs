//! Login event log (04 §7 `login_events`): the audit trail of login attempts, shared by the
//! client and console login flows. Append-only from here; retention is the background
//! jobs' business (M2-20).
//!
//! What a row may hold: who tried (the user id when known, the attempted username, bounded
//! to [`MAX_USERNAME_ATTEMPTED_CHARS`]), how, with what result, from which device and
//! address. Never a password, token or request body.
//!
//! The attempted username and the client device id are what the client typed or sent, so they
//! are stored cleaned like other self-reported text (one line; control, bidi and zero-width
//! characters dropped) and shown as such.

use std::net::IpAddr;

use deskhaven_db::entity::login_event;
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr};

pub use deskhaven_db::entity::login_event::{Client as LoginClient, Method as LoginMethod};

use crate::clock::SharedClock;
use crate::text;

/// Longest stored `username_attempted` (characters; longer input is truncated). Real
/// usernames are at most 64; the rest is room for what people paste into the field.
pub const MAX_USERNAME_ATTEMPTED_CHARS: usize = 128;

/// Longest stored `rustdesk_id` (characters; longer input is truncated).
pub const MAX_RUSTDESK_ID_CHARS: usize = 64;

/// `login_events.result`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginResult {
    Success,
    /// Wrong password, unknown or non-local account: one label, so the log does not
    /// enumerate accounts either.
    InvalidCredentials,
    /// Right password, disabled account.
    Disabled,
    /// Right password, unverified account.
    Unverified,
    /// A failed attempt that started a lockout (the next ones are refused unrecorded).
    Locked,
    /// Right password; a second-factor challenge was issued (the login is not complete).
    SecondFactorRequired,
    /// Right password, but `auth.tfa.enforce` requires a second factor the user has not
    /// enrolled: refused.
    SecondFactorSetupRequired,
    /// Second step: wrong, replayed or already used code.
    InvalidCode,
    /// Second step: unknown, expired, used or exhausted challenge, or one issued to another
    /// username or device.
    InvalidChallenge,
    /// OIDC: the IdP returned an error (cancelled, consent denied) or no code.
    OidcDenied,
    /// OIDC: discovery or the code exchange failed.
    OidcUnavailable,
    /// OIDC: the token response or ID token failed verification.
    OidcInvalidToken,
    /// OIDC: no linked account, and none may be linked or created.
    NoLinkedAccount,
    /// OIDC: creating the account failed.
    ProvisioningFailed,
}

impl LoginResult {
    /// Outcomes that are not a failed login: it succeeded, or the right password is waiting
    /// for its second step, or a policy (not the credentials) stopped it. Every other
    /// outcome, including ones added later, counts as a failure.
    pub const NOT_FAILURES: [Self; 3] = [
        Self::Success,
        Self::SecondFactorRequired,
        Self::SecondFactorSetupRequired,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Disabled => "disabled",
            Self::Unverified => "unverified",
            Self::Locked => "locked",
            Self::SecondFactorRequired => "tfa_required",
            Self::SecondFactorSetupRequired => "tfa_setup_required",
            Self::InvalidCode => "invalid_code",
            Self::InvalidChallenge => "invalid_challenge",
            Self::OidcDenied => "oidc_denied",
            Self::OidcUnavailable => "oidc_unavailable",
            Self::OidcInvalidToken => "oidc_invalid_token",
            Self::NoLinkedAccount => "no_linked_account",
            Self::ProvisioningFailed => "provisioning_failed",
        }
    }
}

impl From<crate::oidc::Failure> for LoginResult {
    fn from(failure: crate::oidc::Failure) -> Self {
        use crate::oidc::Failure;
        match failure {
            Failure::Denied => Self::OidcDenied,
            Failure::Unavailable => Self::OidcUnavailable,
            Failure::InvalidToken => Self::OidcInvalidToken,
            Failure::NoLinkedAccount => Self::NoLinkedAccount,
            Failure::Disabled => Self::Disabled,
            Failure::Unverified => Self::Unverified,
            Failure::ProvisioningFailed => Self::ProvisioningFailed,
        }
    }
}

/// One attempt to record.
#[derive(Debug, Clone)]
pub struct NewLoginEvent<'a> {
    pub user_id: Option<i64>,
    pub username_attempted: &'a str,
    pub method: LoginMethod,
    pub result: LoginResult,
    pub client: LoginClient,
    pub rustdesk_id: Option<&'a str>,
    pub ip: Option<IpAddr>,
}

/// Writes login events. Cheap to clone.
#[derive(Clone)]
pub struct LoginEventLog {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl LoginEventLog {
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    /// Inserts one row stamped with the clock's time.
    pub async fn record(&self, event: &NewLoginEvent<'_>) -> Result<(), DbErr> {
        let now = self.clock.now_ms();
        let created_at = time::from_millis(now)
            .ok_or_else(|| DbErr::Custom(format!("timestamp {now} out of range")))?;
        let row = login_event::ActiveModel {
            user_id: Set(event.user_id),
            username_attempted: Set(text::clean(
                event.username_attempted,
                MAX_USERNAME_ATTEMPTED_CHARS,
                false,
            )),
            method: Set(event.method),
            result: Set(event.result.as_str().to_owned()),
            client: Set(event.client),
            rustdesk_id: Set(event
                .rustdesk_id
                .map(|id| text::clean(id, MAX_RUSTDESK_ID_CHARS, false))),
            ip: Set(event.ip.map(|ip| ip.to_string())),
            created_at: Set(created_at),
            ..Default::default()
        };
        row.insert(&self.db).await.map(drop)
    }
}
