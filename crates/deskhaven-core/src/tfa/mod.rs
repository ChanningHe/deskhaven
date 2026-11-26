//! Replay guard: a code is accepted only for a step strictly after `users.totp_last_step`
//! (conditional UPDATE), so a code works once and an older one never again, across restarts and
//! concurrent requests. These services write only the `totp_*` columns of `users`.

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

pub const ISSUER: &str = "deskhaven";
pub const CHALLENGE_TTL: Duration = Duration::from_secs(5 * 60);
/// The login throttle limits further.
pub const MAX_CHALLENGE_ATTEMPTS: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondFactor {
    NotRequired,
    Required,
    SetupRequired,
}

#[derive(Clone)]
pub struct TfaService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    settings: SettingsService,
    clock: SharedClock,
    /// Cached session principals carry the TOTP flag.
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

    /// Read at each call, so a change applies to the next login.
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

    pub fn enforced_for(&self, role: Role) -> bool {
        enforced(self.inner.settings.snapshot().get(&AUTH_TFA_ENFORCE), role)
    }
}

pub(crate) fn enforced(enforce: TfaEnforce, role: Role) -> bool {
    match enforce {
        TfaEnforce::Off => false,
        TfaEnforce::Admins => matches!(role, Role::Owner | Role::Admin),
        TfaEnforce::All => true,
    }
}

impl Inner {
    fn now(&self) -> TimestampMs {
        self.clock.now()
    }
}

fn seed_context(user_id: i64) -> String {
    format!("users.totp_secret_enc:{user_id}")
}
