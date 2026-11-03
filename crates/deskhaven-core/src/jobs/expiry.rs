//! [`ExpiryCleanup`]: state that has expired or can no longer matter.
//!
//! | Table | Deleted when |
//! |---|---|
//! | `audit_nonces` | received more than [`NONCE_RETENTION`] ago |
//! | `setup_tokens` | used (or retired), or expired |
//! | `device_strategy_keys` | no device row with its RustDesk id, and not written for [`STRATEGY_KEYS_GRACE`] (deleting the device writes it: see `devices::strategy_keys`) |
//! | `sessions` | revoked, or expired (absolute or idle): [`SessionService::cleanup`] |
//! | `oidc_flows` | expired: [`OidcService::cleanup_flows`] |
//! | `auth_challenges` | expired or consumed: [`TfaService::cleanup_challenges`] |
//! | `auth_lockouts` | no longer affecting any decision: [`LoginThrottle::cleanup`] |
//!
//! The first three are batched here ([`Purge`]); the services' own cleanups are small
//! tables bounded by the login rate limits.
//!
//! [`SessionService::cleanup`]: crate::sessions::SessionService::cleanup
//! [`OidcService::cleanup_flows`]: crate::oidc::OidcService::cleanup_flows
//! [`TfaService::cleanup_challenges`]: crate::tfa::TfaService::cleanup_challenges
//! [`LoginThrottle::cleanup`]: crate::auth::limiter::LoginThrottle::cleanup

use std::sync::Arc;
use std::time::Duration;

use deskhaven_db::entity::{audit_nonce, device, device_strategy_key, setup_token};
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, DbErr};

use super::purge::{Doomed, Plain, Purge};
use super::{chain, cutoff};
use crate::oidc::OidcError;
use crate::sessions::SessionError;
use crate::state::AppState;
use crate::tfa::TfaError;

/// Audit nonces are kept this long (04 §7), far past the client's 120 s retry window.
pub const NONCE_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
/// A `device_strategy_keys` row is deleted once its RustDesk id has had no device row for
/// this long. A deleted device's client that comes back later registers as a new device and
/// is not sent `""` for the options it still holds from before.
pub const STRATEGY_KEYS_GRACE: Duration = Duration::from_secs(90 * 24 * 60 * 60);

/// Rows deleted by one [`ExpiryCleanup::run`], per table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    pub audit_nonces: u64,
    pub setup_tokens: u64,
    pub strategy_keys: u64,
    pub sessions: u64,
    pub oidc_flows: u64,
    pub auth_challenges: u64,
    pub auth_lockouts: u64,
    /// Steps that failed (logged; retried by the next run).
    pub failed: usize,
}

impl ExpiryReport {
    pub fn total(&self) -> u64 {
        self.audit_nonces
            + self.setup_tokens
            + self.strategy_keys
            + self.sessions
            + self.oidc_flows
            + self.auth_challenges
            + self.auth_lockouts
    }
}

/// Why a step failed.
#[derive(Debug, thiserror::Error)]
enum StepError {
    #[error(transparent)]
    Database(#[from] DbErr),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Oidc(#[from] OidcError),
    #[error(transparent)]
    Tfa(#[from] TfaError),
}

/// Counts failed steps, logging each.
#[derive(Default)]
struct Tally {
    failed: usize,
}

impl Tally {
    fn take<E: Into<StepError>>(&mut self, table: &'static str, result: Result<u64, E>) -> u64 {
        match result.map_err(Into::into) {
            Ok(deleted) => deleted,
            Err(err) => {
                tracing::warn!(table, error = %chain(&err), "expiry cleanup failed; retrying next run");
                self.failed += 1;
                0
            }
        }
    }
}

/// The expiry cleanup job.
#[derive(Clone)]
pub struct ExpiryCleanup {
    state: Arc<AppState>,
    purge: Purge,
}

impl ExpiryCleanup {
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            purge: Purge::default(),
        }
    }

    pub fn with_purge(self, purge: Purge) -> Self {
        Self { purge, ..self }
    }

    /// Runs every step (see the module documentation); a failed step does not stop the
    /// others.
    pub async fn run(&self) -> ExpiryReport {
        let now_ms = self.state.clock.now_ms();
        let s = &self.state;
        let mut tally = Tally::default();
        let audit_nonces = tally.take("audit_nonces", self.nonces(now_ms).await);
        let setup_tokens = tally.take("setup_tokens", self.setup_tokens(now_ms).await);
        let strategy_keys = tally.take("device_strategy_keys", self.strategy_keys(now_ms).await);
        let sessions = tally.take("sessions", s.sessions.cleanup().await);
        let oidc_flows = tally.take("oidc_flows", s.oidc.cleanup_flows().await);
        let auth_challenges = tally.take("auth_challenges", s.tfa.cleanup_challenges().await);
        let auth_lockouts = tally.take("auth_lockouts", s.login_throttle.cleanup().await);
        ExpiryReport {
            audit_nonces,
            setup_tokens,
            strategy_keys,
            sessions,
            oidc_flows,
            auth_challenges,
            auth_lockouts,
            failed: tally.failed,
        }
    }

    async fn nonces(&self, now_ms: i64) -> Result<u64, DbErr> {
        use audit_nonce::Column as C;
        let before = cutoff(now_ms, NONCE_RETENTION)?;
        let doomed = Doomed::<audit_nonce::Entity> {
            key: C::Nonce,
            order: C::ReceivedAt,
            cond: Condition::all().add(C::ReceivedAt.lt(before)),
        };
        self.purge.run(&self.state.db, &doomed, &mut Plain).await
    }

    async fn setup_tokens(&self, now_ms: i64) -> Result<u64, DbErr> {
        use setup_token::Column as C;
        let now = cutoff(now_ms, Duration::ZERO)?;
        let doomed = Doomed::<setup_token::Entity> {
            key: C::Id,
            order: C::Id,
            cond: Condition::any()
                .add(C::UsedAt.is_not_null())
                .add(C::ExpiresAt.lte(now)),
        };
        self.purge.run(&self.state.db, &doomed, &mut Plain).await
    }

    async fn strategy_keys(&self, now_ms: i64) -> Result<u64, DbErr> {
        use device_strategy_key::Column as C;
        let before = cutoff(now_ms, STRATEGY_KEYS_GRACE)?;
        let device_row = Query::select()
            .expr(Expr::val(1))
            .from(device::Entity)
            .and_where(
                Expr::col((device::Entity, device::Column::RustdeskId))
                    .equals((device_strategy_key::Entity, C::RustdeskId)),
            )
            .to_owned();
        let doomed = Doomed::<device_strategy_key::Entity> {
            key: C::RustdeskId,
            order: C::UpdatedAt,
            cond: Condition::all()
                .add(C::UpdatedAt.lt(before))
                .add(Expr::not_exists(device_row)),
        };
        self.purge.run(&self.state.db, &doomed, &mut Plain).await
    }
}
