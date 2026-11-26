//! Every operation is one write transaction over the user row, so concurrent calls for one user
//! apply one after another.

use std::fmt;

use deskhaven_db::begin_write;
use deskhaven_db::entity::{auth_challenge, recovery_code, user};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect,
};
use zeroize::Zeroizing;

use super::totp::{self, TotpSecret};
use super::{ISSUER, Inner, RecoveryCodes, TfaError, TfaService, recovery, seed_context};
use crate::console_audit::{self, NewConsoleAudit, TxGuard};
use crate::users::User;

/// `Debug` never shows the seed.
pub struct Enrolment {
    pub secret: Zeroizing<String>,
    pub uri: Zeroizing<String>,
}

impl fmt::Debug for Enrolment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Enrolment(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TfaStatus {
    pub enabled: bool,
    pub pending_enrolment: bool,
    pub recovery_codes_left: u64,
}

impl TfaService {
    pub async fn begin_enrolment(&self, user_id: i64) -> Result<Enrolment, TfaError> {
        self.begin(user_id, None).await
    }

    pub async fn begin_enrolment_audited(
        &self,
        user_id: i64,
        audit: &NewConsoleAudit,
    ) -> Result<Enrolment, TfaError> {
        self.begin(user_id, Some(audit)).await
    }

    async fn begin(
        &self,
        user_id: i64,
        audit: Option<&NewConsoleAudit>,
    ) -> Result<Enrolment, TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let user = locked_user(&txn, user_id).await?;
        if user.totp_enabled_at.is_some() {
            return Err(TfaError::AlreadyEnabled);
        }
        let secret = TotpSecret::generate()?;
        let sealed = inner.crypto.seal(&seed_context(user_id), secret.bytes())?;
        set_totp(&txn, inner, user_id, Some(sealed), None, None).await?;
        record(&txn, inner, audit).await?;
        txn.commit().await?;
        Ok(Enrolment {
            secret: secret.to_base32(),
            uri: totp::provisioning_uri(&secret, ISSUER, &user.username),
        })
    }

    /// The confirming code's step is recorded, so it cannot log in.
    pub async fn confirm_enrolment(
        &self,
        user_id: i64,
        code: &str,
    ) -> Result<RecoveryCodes, TfaError> {
        self.confirm(user_id, code, None).await
    }

    pub async fn confirm_enrolment_audited(
        &self,
        user_id: i64,
        code: &str,
        audit: &NewConsoleAudit,
    ) -> Result<RecoveryCodes, TfaError> {
        self.confirm(user_id, code, Some(audit)).await
    }

    async fn confirm(
        &self,
        user_id: i64,
        code: &str,
        audit: Option<&NewConsoleAudit>,
    ) -> Result<RecoveryCodes, TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let user = locked_user(&txn, user_id).await?;
        if user.totp_enabled_at.is_some() {
            return Err(TfaError::AlreadyEnabled);
        }
        let sealed = user.totp_secret_enc.ok_or(TfaError::NoPendingEnrolment)?;
        let secret = inner.open_seed(user_id, &sealed)?;
        let step = totp::matching_step(&secret, code, inner.clock.now_ms(), None)
            .ok_or(TfaError::InvalidCode)?;
        let now = inner.now();
        set_totp(&txn, inner, user_id, Some(sealed), Some(now), Some(step)).await?;
        let codes = replace_recovery_codes(&txn, inner, user_id).await?;
        record(&txn, inner, audit).await?;
        txn.commit().await?;
        inner.users.second_factor_changed(user_id);
        Ok(codes)
    }

    /// Also deletes every unconsumed login challenge of the user. Idempotent.
    pub async fn disable(&self, user_id: i64) -> Result<(), TfaError> {
        self.disable_guarded(user_id, TxGuard::none()).await
    }

    pub async fn disable_guarded(
        &self,
        user_id: i64,
        guard: TxGuard<User>,
    ) -> Result<(), TfaError> {
        self.disable_checked(user_id, None, guard).await
    }

    /// The owner proves possession: the code is spent in the same transaction as the removal, and a
    /// wrong one changes nothing.
    pub async fn disable_verified(
        &self,
        user_id: i64,
        code: &str,
        guard: TxGuard<User>,
    ) -> Result<(), TfaError> {
        self.disable_checked(user_id, Some(code), guard).await
    }

    async fn disable_checked(
        &self,
        user_id: i64,
        code: Option<&str>,
        guard: TxGuard<User>,
    ) -> Result<(), TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let current = locked_user(&txn, user_id).await?;
        if let Some(code) = code {
            if current.totp_enabled_at.is_none() {
                return Err(TfaError::NotEnabled);
            }
            if !inner.spend_code(&txn, &current, code).await? {
                return Err(TfaError::InvalidCode);
            }
        }
        let cleared = user::Model {
            totp_secret_enc: None,
            totp_enabled_at: None,
            totp_last_step: None,
            ..current.clone()
        };
        let (before, after) = (User::from(current), User::from(cleared));
        guard.check(&txn, Some(&before), Some(&after)).await?;
        set_totp(&txn, inner, user_id, None, None, None).await?;
        recovery_code::Entity::delete_many()
            .filter(recovery_code::Column::UserId.eq(user_id))
            .exec(&txn)
            .await?;
        auth_challenge::Entity::delete_many()
            .filter(auth_challenge::Column::UserId.eq(user_id))
            .filter(auth_challenge::Column::ConsumedAt.is_null())
            .exec(&txn)
            .await?;
        txn.commit().await?;
        inner.users.second_factor_changed(user_id);
        Ok(())
    }

    pub async fn regenerate_recovery_codes(&self, user_id: i64) -> Result<RecoveryCodes, TfaError> {
        self.regenerate(user_id, None, None).await
    }

    /// The code is spent in the same transaction; a wrong one changes nothing.
    pub async fn regenerate_recovery_codes_verified(
        &self,
        user_id: i64,
        code: &str,
        audit: &NewConsoleAudit,
    ) -> Result<RecoveryCodes, TfaError> {
        self.regenerate(user_id, Some(code), Some(audit)).await
    }

    async fn regenerate(
        &self,
        user_id: i64,
        code: Option<&str>,
        audit: Option<&NewConsoleAudit>,
    ) -> Result<RecoveryCodes, TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let user = locked_user(&txn, user_id).await?;
        if user.totp_enabled_at.is_none() {
            return Err(TfaError::NotEnabled);
        }
        if let Some(code) = code
            && !inner.spend_code(&txn, &user, code).await?
        {
            return Err(TfaError::InvalidCode);
        }
        let codes = replace_recovery_codes(&txn, inner, user_id).await?;
        record(&txn, inner, audit).await?;
        txn.commit().await?;
        Ok(codes)
    }

    pub async fn status(&self, user_id: i64) -> Result<TfaStatus, TfaError> {
        let db = &self.inner.db;
        let user = user::Entity::find_by_id(user_id)
            .one(db)
            .await?
            .ok_or(TfaError::NotFound)?;
        let recovery_codes_left = recovery_code::Entity::find()
            .filter(recovery_code::Column::UserId.eq(user_id))
            .filter(recovery_code::Column::UsedAt.is_null())
            .count(db)
            .await?;
        Ok(TfaStatus {
            enabled: user.totp_enabled_at.is_some(),
            pending_enrolment: user.totp_enabled_at.is_none() && user.totp_secret_enc.is_some(),
            recovery_codes_left,
        })
    }
}

impl Inner {
    pub(super) fn open_seed(&self, user_id: i64, sealed: &str) -> Result<TotpSecret, TfaError> {
        let bytes = self
            .crypto
            .open(&seed_context(user_id), sealed)
            .map_err(|err| {
                tracing::error!(user_id, error = %err, "stored TOTP seed does not open");
                TfaError::Corrupt
            })?;
        TotpSecret::from_bytes(Zeroizing::new(bytes.to_vec())).ok_or(TfaError::Corrupt)
    }
}

async fn record(
    txn: &DatabaseTransaction,
    inner: &Inner,
    audit: Option<&NewConsoleAudit>,
) -> Result<(), TfaError> {
    if let Some(entry) = audit {
        console_audit::insert(txn, entry, inner.clock.now_ms()).await?;
    }
    Ok(())
}

async fn locked_user(txn: &DatabaseTransaction, user_id: i64) -> Result<user::Model, TfaError> {
    user::Entity::find_by_id(user_id)
        .lock_exclusive()
        .one(txn)
        .await?
        .ok_or(TfaError::NotFound)
}

async fn set_totp(
    txn: &DatabaseTransaction,
    inner: &Inner,
    user_id: i64,
    sealed: Option<String>,
    enabled_at: Option<deskhaven_db::time::TimestampMs>,
    last_step: Option<i64>,
) -> Result<(), TfaError> {
    user::Entity::update_many()
        .col_expr(user::Column::TotpSecretEnc, Expr::value(sealed))
        .col_expr(user::Column::TotpEnabledAt, Expr::value(enabled_at))
        .col_expr(user::Column::TotpLastStep, Expr::value(last_step))
        .col_expr(user::Column::UpdatedAt, Expr::value(inner.now()))
        .filter(user::Column::Id.eq(user_id))
        .exec(txn)
        .await?;
    Ok(())
}

async fn replace_recovery_codes(
    txn: &DatabaseTransaction,
    inner: &Inner,
    user_id: i64,
) -> Result<RecoveryCodes, TfaError> {
    recovery_code::Entity::delete_many()
        .filter(recovery_code::Column::UserId.eq(user_id))
        .exec(txn)
        .await?;
    let (codes, hashes) = recovery::generate(&inner.crypto, user_id)?;
    let now = inner.now();
    let rows = hashes
        .into_iter()
        .map(|code_hash| recovery_code::ActiveModel {
            user_id: Set(user_id),
            code_hash: Set(code_hash),
            used_at: Set(None),
            created_at: Set(now),
            ..Default::default()
        });
    recovery_code::Entity::insert_many(rows).exec(txn).await?;
    Ok(codes)
}
