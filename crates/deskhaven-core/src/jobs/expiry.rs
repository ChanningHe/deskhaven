use std::sync::Arc;
use std::time::Duration;

use deskhaven_db::entity::{audit_nonce, device, device_strategy_key, setup_token};
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, DbErr};

use super::purge::{Doomed, Plain, Purge};
use super::{chain, cutoff};
use crate::sessions::SessionError;
use crate::state::AppState;
use crate::{oidc, tfa};

/// Far past the client's 120 s retry window.
pub const NONCE_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);
/// A deleted device's client that comes back later registers as new and is not sent `""` for the
/// options it still holds.
pub const STRATEGY_KEYS_GRACE: Duration = Duration::from_secs(90 * 24 * 60 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpiryReport {
    pub audit_nonces: u64,
    pub setup_tokens: u64,
    pub strategy_keys: u64,
    pub sessions: u64,
    pub oidc_flows: u64,
    pub auth_challenges: u64,
    pub auth_lockouts: u64,
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

#[derive(Debug, thiserror::Error)]
enum StepError {
    #[error(transparent)]
    Database(#[from] DbErr),
    #[error(transparent)]
    Session(#[from] SessionError),
}

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

    pub async fn run(&self) -> ExpiryReport {
        let now_ms = self.state.clock.now_ms();
        let s = &self.state;
        let mut tally = Tally::default();
        let audit_nonces = tally.take("audit_nonces", self.nonces(now_ms).await);
        let setup_tokens = tally.take("setup_tokens", self.setup_tokens(now_ms).await);
        let strategy_keys = tally.take("device_strategy_keys", self.strategy_keys(now_ms).await);
        let sessions = tally.take("sessions", s.sessions.cleanup().await);
        let oidc_flows = tally.take("oidc_flows", self.oidc_flows(now_ms).await);
        let auth_challenges = tally.take("auth_challenges", self.challenges(now_ms).await);
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

    async fn oidc_flows(&self, now_ms: i64) -> Result<u64, DbErr> {
        let doomed = oidc::expired_flows(cutoff(now_ms, Duration::ZERO)?);
        self.purge.run(&self.state.db, &doomed, &mut Plain).await
    }

    async fn challenges(&self, now_ms: i64) -> Result<u64, DbErr> {
        let doomed = tfa::finished_challenges(cutoff(now_ms, Duration::ZERO)?);
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
