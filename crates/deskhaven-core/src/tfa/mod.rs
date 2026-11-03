//! Two-factor authentication: TOTP with recovery codes, and the login challenge of the
//! two-step login (06 §3.3, 04 §3 `users.totp_*`, `recovery_codes`, `auth_challenges`).
//!
//! # Storage
//!
//! - Seed: `users.totp_secret_enc`, sealed with [`SecretBox`] under the context
//!   `users.totp_secret_enc:<user id>` (a ciphertext copied to another row does not open).
//!   `totp_enabled_at` NULL with a seed present = enrolment pending (not yet confirmed).
//! - Replay guard: `users.totp_last_step`, the last accepted time step. A code is accepted
//!   only for a step strictly after it (conditional UPDATE), so a code works once and an
//!   older one never again, across restarts and concurrent requests.
//! - Recovery codes: `recovery_codes.code_hash` = `SecretBox::mac` (see `recovery.rs`),
//!   single use through `UPDATE … SET used_at WHERE used_at IS NULL`.
//! - Login challenge: `auth_challenges` with `secret_hash` = SHA-256 of the 256-bit secret
//!   returned to the client (the session-token format), `kind = totp`, `attempts`
//!   (claimed before verification, capped at [`MAX_CHALLENGE_ATTEMPTS`]), `expires_at`
//!   ([`CHALLENGE_TTL`]), `consumed_at` (single use) and `context` (JSON: the client device
//!   it was issued to, the login throttle's pending subject and the password's credential
//!   stamp; written and read only here).
//!
//! These services write only the `totp_*` columns (and `updated_at`) of `users`; every
//! other column belongs to [`UserService`](crate::users::UserService).
//!
//! # Login protocol (with `auth::limiter`)
//!
//! Step 1, after the password verified: [`TfaService::requirement`]. `Required` ->
//! `LoginThrottle::on_password_ok_pending_2fa` and [`TfaService::issue_challenge`] with the
//! returned subject; no session. `SetupRequired` (`auth.tfa.enforce` covers the user, who
//! has no second factor): refuse the login, enrolment happens in the console.
//! Step 2: `LoginThrottle::pre_check` with the **same username**, then
//! [`TfaService::verify_challenge`]; success -> `LoginThrottle::complete` with the
//! challenge's subject and the session from the challenge's credential stamp; failure ->
//! `on_failure`.
//!
//! Enrolment and management ([`TfaService::begin_enrolment`] and friends) are the console's
//! (M3); they re-authenticate the user themselves.

mod challenge;
mod enrol;
mod error;
mod recovery;
pub mod totp;
mod verify;

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::{AUTH_TFA_ENFORCE, TfaEnforce};
use deskhaven_db::time::TimestampMs;
use sea_orm::DatabaseConnection;

pub(crate) use challenge::finished_challenges;
pub use challenge::{ChallengeSecret, LoginBinding, PendingLogin, SecondStep};
pub use enrol::{Enrolment, TfaStatus};
pub use error::{ChallengeError, TfaError};
pub use recovery::{RECOVERY_CODE_CHARS, RECOVERY_CODE_COUNT, RecoveryCodes};
pub use verify::{Passed, SecondFactorMethod};

use crate::clock::SharedClock;
use crate::crypto::SecretBox;
use crate::settings::SettingsService;
use crate::users::{Role, User, UserService};

/// `issuer` of provisioning URIs (the account name shown in authenticator apps).
pub const ISSUER: &str = "deskhaven";
/// Lifetime of a login challenge.
pub const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
/// Codes that may be tried against one challenge (the login throttle limits further).
pub const MAX_CHALLENGE_ATTEMPTS: i32 = 5;

/// What a login needs after the password verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondFactor {
    /// No second factor: the login is complete.
    NotRequired,
    /// TOTP is enabled: issue a challenge.
    Required,
    /// `auth.tfa.enforce` covers the user, who has not enrolled: refuse the login.
    SetupRequired,
}

/// TOTP enrolment, recovery codes and login challenges. Cheap to clone.
#[derive(Clone)]
pub struct TfaService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    settings: SettingsService,
    clock: SharedClock,
    /// Told when TOTP is enabled or disabled (cached session principals carry it).
    users: UserService,
}

impl TfaService {
    pub fn new(
        db: DatabaseConnection,
        crypto: Arc<SecretBox>,
        settings: SettingsService,
        clock: SharedClock,
        users: UserService,
    ) -> Self {
        let inner = Inner {
            db,
            crypto,
            settings,
            clock,
            users,
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    /// The second factor `user` needs, from its enrolment and `auth.tfa.enforce` (read at
    /// each call, so a change applies to the next login).
    pub fn requirement(&self, user: &User) -> SecondFactor {
        if user.totp_enabled {
            return SecondFactor::Required;
        }
        if self.enforced_for(user.role) {
            SecondFactor::SetupRequired
        } else {
            SecondFactor::NotRequired
        }
    }

    /// Whether `auth.tfa.enforce` (read at each call) requires a second factor of `role`.
    pub fn enforced_for(&self, role: Role) -> bool {
        match self.inner.settings.snapshot().get(&AUTH_TFA_ENFORCE) {
            TfaEnforce::Off => false,
            TfaEnforce::Admins => matches!(role, Role::Owner | Role::Admin),
            TfaEnforce::All => true,
        }
    }
}

impl Inner {
    fn now(&self) -> TimestampMs {
        self.clock.now()
    }
}

/// Sealing context of a user's seed.
fn seed_context(user_id: i64) -> String {
    format!("users.totp_secret_enc:{user_id}")
}
