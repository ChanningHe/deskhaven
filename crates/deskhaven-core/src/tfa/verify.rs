//! Either both happen or neither: a wrong code leaves the challenge usable, a challenge consumed
//! concurrently leaves the code unspent.

use deskhaven_db::begin_write;
use deskhaven_db::entity::{auth_challenge, recovery_code, user};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, DatabaseTransaction, EntityTrait, QueryFilter, QuerySelect};

use super::challenge::{Bound, SecondStep};
use super::{ChallengeError, Inner, TfaError, TfaService, recovery, totp};
use crate::auth::limiter::PendingSubject;
use crate::users::{CredentialStamp, User};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondFactorMethod {
    Totp,
    Recovery,
}

impl SecondFactorMethod {
    pub fn of_code(code: &str) -> Self {
        if totp::is_code_shaped(code) {
            Self::Totp
        } else {
            Self::Recovery
        }
    }
}

#[derive(Debug)]
pub struct Passed {
    pub user: User,
    pub stamp: CredentialStamp,
    pub lockout: PendingSubject,
    pub method: SecondFactorMethod,
}

impl TfaService {
    pub async fn verify_challenge(&self, step: &SecondStep<'_>) -> Result<Passed, ChallengeError> {
        let inner = &self.inner;
        let (challenge, bound) = inner.claim(step).await?;
        if challenge.kind != auth_challenge::Kind::Totp {
            return Err(ChallengeError::Invalid);
        }
        let method = SecondFactorMethod::of_code(step.code);
        let txn = begin_write(&inner.db).await?;
        let user = user::Entity::find_by_id(challenge.user_id)
            .lock_exclusive()
            .one(&txn)
            .await?
            .filter(|u| u.totp_enabled_at.is_some())
            // Deleted, or 2FA turned off since step 1: the challenge is moot.
            .ok_or(ChallengeError::Invalid)?;
        let spent = match method {
            SecondFactorMethod::Totp => inner.spend_totp(&txn, &user, step.code).await?,
            SecondFactorMethod::Recovery => inner.spend_recovery(&txn, user.id, step.code).await?,
        };
        if !spent {
            return Err(ChallengeError::WrongCode);
        }
        if !consume(&txn, inner, challenge.id).await? {
            return Err(ChallengeError::Invalid);
        }
        txn.commit().await?;
        let Bound { lockout, stamp, .. } = bound;
        Ok(Passed {
            user: user.into(),
            stamp,
            lockout,
            method,
        })
    }

    /// The code is spent (anti-replay) in its own transaction.
    pub async fn prove_possession(&self, user_id: i64, code: &str) -> Result<(), TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let user = user::Entity::find_by_id(user_id)
            .lock_exclusive()
            .one(&txn)
            .await?
            .ok_or(TfaError::NotFound)?;
        if user.totp_enabled_at.is_none() {
            return Err(TfaError::NotEnabled);
        }
        if !inner.spend_code(&txn, &user, code).await? {
            return Err(TfaError::InvalidCode);
        }
        txn.commit().await?;
        Ok(())
    }
}

impl Inner {
    pub(super) async fn spend_code(
        &self,
        txn: &DatabaseTransaction,
        user: &user::Model,
        code: &str,
    ) -> Result<bool, TfaError> {
        let spent = match SecondFactorMethod::of_code(code) {
            SecondFactorMethod::Totp => self.spend_totp(txn, user, code).await,
            SecondFactorMethod::Recovery => self.spend_recovery(txn, user.id, code).await,
        };
        spent.map_err(|err| match err {
            ChallengeError::Internal(inner) => inner,
            // The spend functions report only storage failures as errors.
            ChallengeError::Invalid | ChallengeError::WrongCode => TfaError::InvalidCode,
        })
    }

    /// The conditional UPDATE keeps a concurrent request from using the same step.
    async fn spend_totp(
        &self,
        txn: &DatabaseTransaction,
        user: &user::Model,
        code: &str,
    ) -> Result<bool, ChallengeError> {
        let Some(sealed) = user.totp_secret_enc.as_deref() else {
            return Ok(false);
        };
        let secret = self.open_seed(user.id, sealed)?;
        let now = self.clock.now_ms();
        let Some(step) = totp::matching_step(&secret, code, now, user.totp_last_step) else {
            return Ok(false);
        };
        let newer = Condition::any()
            .add(user::Column::TotpLastStep.is_null())
            .add(user::Column::TotpLastStep.lt(step));
        let result = user::Entity::update_many()
            .col_expr(user::Column::TotpLastStep, Expr::value(step))
            .filter(user::Column::Id.eq(user.id))
            .filter(newer)
            .exec(txn)
            .await?;
        Ok(result.rows_affected == 1)
    }

    async fn spend_recovery(
        &self,
        txn: &DatabaseTransaction,
        user_id: i64,
        code: &str,
    ) -> Result<bool, ChallengeError> {
        let Some(normalized) = recovery::normalize(code) else {
            return Ok(false);
        };
        let candidates = recovery::candidates(&self.crypto, user_id, &normalized);
        let result = recovery_code::Entity::update_many()
            .col_expr(recovery_code::Column::UsedAt, Expr::value(self.now()))
            .filter(recovery_code::Column::UserId.eq(user_id))
            .filter(recovery_code::Column::CodeHash.is_in(candidates))
            .filter(recovery_code::Column::UsedAt.is_null())
            .exec(txn)
            .await?;
        Ok(result.rows_affected > 0)
    }
}

/// Single use: only the first consumer of a challenge gets `true`.
async fn consume(
    txn: &DatabaseTransaction,
    inner: &Inner,
    challenge_id: i64,
) -> Result<bool, ChallengeError> {
    let result = auth_challenge::Entity::update_many()
        .col_expr(auth_challenge::Column::ConsumedAt, Expr::value(inner.now()))
        .filter(auth_challenge::Column::Id.eq(challenge_id))
        .filter(auth_challenge::Column::ConsumedAt.is_null())
        .exec(txn)
        .await?;
    Ok(result.rows_affected == 1)
}
