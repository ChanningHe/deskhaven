//! A user's OIDC identities (`user_identities`, M4-09): listing, and removal by the account
//! holder or an administrator. Who may remove what is the caller's [`TxGuard`]; this
//! module keeps one rule of its own: an `oidc` account keeps its last identity (it has no
//! password, so without one nobody could sign in to it).

use deskhaven_db::begin_write;
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{oidc_provider, user};
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, IntoActiveModel,
    ModelTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

use super::OidcService;
use super::error::IdentityError;
use crate::console_audit::TxGuard;
use crate::users::{AuthSource, User};

/// One identity of an account, with its provider's names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub id: i64,
    pub user_id: i64,
    pub provider_id: i64,
    pub provider_name: String,
    pub provider_display_name: String,
    /// The provider's `sub`.
    pub subject: String,
    /// The last address the IdP asserted verified, if any.
    pub email: Option<String>,
    pub linked_via: LinkedVia,
    pub created_at: TimestampMs,
}

impl Identity {
    pub(crate) fn of(row: user_identity::Model, provider: &oidc_provider::Model) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            provider_id: row.provider_id,
            provider_name: provider.name.clone(),
            provider_display_name: provider.display_name.clone(),
            subject: row.subject,
            email: row.email,
            linked_via: row.linked_via,
            created_at: row.created_at,
        }
    }
}

/// What a guard of an identity change sees: the account and the identity (`before` of a
/// removal, `after` of a link).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityChange {
    pub user: User,
    pub identity: Identity,
}

impl OidcService {
    /// The identities of account `user_id`, oldest first.
    pub async fn identities_of(&self, user_id: i64) -> Result<Vec<Identity>, IdentityError> {
        let rows = user_identity::Entity::find()
            .filter(user_identity::Column::UserId.eq(user_id))
            .order_by_asc(user_identity::Column::Id)
            .find_also_related(oidc_provider::Entity)
            .all(&self.inner.db)
            .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(row, provider)| provider.map(|p| Identity::of(row, &p)))
            .collect())
    }

    /// Removes identity `id`; with `of_user`, only when it is that account's (the
    /// self-service path: another account's identity is [`IdentityError::NotFound`]).
    /// `guard` sees the account and the identity as `before` and records the audit row.
    pub async fn unlink_identity(
        &self,
        id: i64,
        of_user: Option<i64>,
        guard: TxGuard<IdentityChange>,
    ) -> Result<(), IdentityError> {
        let tx = begin_write(&self.inner.db).await?;
        let row = user_identity::Entity::find_by_id(id)
            .one(&tx)
            .await?
            .filter(|row| of_user.is_none_or(|owner| owner == row.user_id))
            .ok_or(IdentityError::NotFound)?;
        let account = locked_account(&tx, row.user_id)
            .await?
            .ok_or(IdentityError::NotFound)?;
        if account.auth_source == AuthSource::Oidc && identity_count(&tx, account.id).await? <= 1 {
            return Err(IdentityError::LastIdentity);
        }
        let provider = row
            .find_related(oidc_provider::Entity)
            .one(&tx)
            .await?
            .ok_or(IdentityError::NotFound)?;
        let change = IdentityChange {
            user: User::from(account.clone()),
            identity: Identity::of(row.clone(), &provider),
        };
        guard.check(&tx, Some(&change), None).await?;
        let by_email = row.linked_via == LinkedVia::Email;
        row.delete(&tx).await?;
        if by_email && account.email_verified {
            self.unverify_email(&tx, account).await?;
        }
        tx.commit().await?;
        tracing::info!(user_id = change.user.id, provider = %provider.name, "OIDC identity unlinked");
        Ok(())
    }

    /// An identity linked by email would be linked again at its next login while the email
    /// conditions hold: removing one also marks the account's address unverified (an
    /// administrator can mark it verified again; the holder can link explicitly).
    async fn unverify_email<C: ConnectionTrait>(
        &self,
        conn: &C,
        account: user::Model,
    ) -> Result<(), DbErr> {
        let mut row = account.into_active_model();
        row.email_verified = Set(false);
        row.updated_at = Set(self.inner.clock.now());
        row.update(conn).await?;
        Ok(())
    }
}

/// The account row, write-locked until the transaction ends (PostgreSQL; SQLite's write
/// transaction already excludes other writers).
pub(crate) async fn locked_account<C: ConnectionTrait>(
    conn: &C,
    user_id: i64,
) -> Result<Option<user::Model>, DbErr> {
    user::Entity::find_by_id(user_id)
        .lock_exclusive()
        .one(conn)
        .await
}

async fn identity_count<C: ConnectionTrait>(conn: &C, user_id: i64) -> Result<u64, DbErr> {
    user_identity::Entity::find()
        .filter(user_identity::Column::UserId.eq(user_id))
        .count(conn)
        .await
}
