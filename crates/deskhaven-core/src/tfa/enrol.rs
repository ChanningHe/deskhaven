//! TOTP enrolment and management: begin (new pending seed) -> confirm with a code
//! (enabled, recovery codes issued) -> disable; recovery code regeneration.
//!
//! Every operation is one write transaction over the user row (locked on PostgreSQL;
//! SQLite's `BEGIN IMMEDIATE` excludes other writers), so concurrent calls for one user
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
use crate::console_audit::{self, NewConsoleAudit};

/// A pending enrolment, to show its owner once (as text and as a QR code of `uri`).
/// `Debug` never shows the seed.
pub struct Enrolment {
    /// The seed in base32, for manual entry.
    pub secret: Zeroizing<String>,
    /// `otpauth://` provisioning URI.
    pub uri: Zeroizing<String>,
}

impl fmt::Debug for Enrolment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Enrolment(<redacted>)")
    }
}

/// A user's 2FA state, for the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TfaStatus {
    pub enabled: bool,
    /// A seed waits for confirmation.
    pub pending_enrolment: bool,
    pub recovery_codes_left: u64,
}

impl TfaService {
    /// Generates a new seed and stores it, sealed, as a pending enrolment (replacing any
    /// earlier pending one). [`TfaError::AlreadyEnabled`] while TOTP is enabled.
    pub async fn begin_enrolment(&self, user_id: i64) -> Result<Enrolment, TfaError> {
        self.begin(user_id, None).await
    }

    /// [`begin_enrolment`](Self::begin_enrolment), recording `audit` in its transaction.
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

    /// Enables TOTP when `code` matches the pending seed, and issues fresh recovery codes
    /// (returned once). The confirming code's step is recorded, so it cannot log in.
    pub async fn confirm_enrolment(
        &self,
        user_id: i64,
        code: &str,
    ) -> Result<RecoveryCodes, TfaError> {
        self.confirm(user_id, code, None).await
    }

    /// [`confirm_enrolment`](Self::confirm_enrolment), recording `audit` in its transaction.
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
        Ok(codes)
    }

    /// Turns TOTP off (or cancels a pending enrolment): forgets the seed, deletes the
    /// recovery codes and every unconsumed login challenge of the user. Idempotent.
    pub async fn disable(&self, user_id: i64) -> Result<(), TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        locked_user(&txn, user_id).await?;
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
        Ok(())
    }

    /// Replaces every recovery code (used or not) with fresh ones, returned once.
    pub async fn regenerate_recovery_codes(&self, user_id: i64) -> Result<RecoveryCodes, TfaError> {
        let inner = &self.inner;
        let txn = begin_write(&inner.db).await?;
        let user = locked_user(&txn, user_id).await?;
        if user.totp_enabled_at.is_none() {
            return Err(TfaError::NotEnabled);
        }
        let codes = replace_recovery_codes(&txn, inner, user_id).await?;
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
    /// Opens a sealed seed; a seed that does not open or decode is [`TfaError::Corrupt`].
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

/// Writes `audit` (when given) in the operation's transaction.
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

/// The user row, write-locked until the transaction ends (PostgreSQL).
async fn locked_user(txn: &DatabaseTransaction, user_id: i64) -> Result<user::Model, TfaError> {
    user::Entity::find_by_id(user_id)
        .lock_exclusive()
        .one(txn)
        .await?
        .ok_or(TfaError::NotFound)
}

/// Writes the three `totp_*` columns (and `updated_at`).
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
