//! Session creation: one write transaction that re-checks the login's credential stamp and
//! inserts the hashed token; and the rotation of a console session on a privilege change.

use std::net::IpAddr;

use deskhaven_db::begin_write;
use deskhaven_db::entity::session;
use deskhaven_db::entity::user::{self, Status};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, DbErr, EntityTrait, QuerySelect};

use super::model::console_info;
use super::revoke::revoke_rows;
use super::{
    ClientDevice, Core, DeviceBinding, Session, SessionError, SessionKind, SessionService,
    SessionToken, detached, timestamp,
};
use crate::users::CredentialStamp;

/// A session row before insertion.
struct NewSession {
    user_id: i64,
    kind: SessionKind,
    device: Option<DeviceBinding>,
    device_info: Option<String>,
    ip: Option<IpAddr>,
}

impl SessionService {
    /// Issues a client session bound to `device` for the user a local login verified.
    /// [`SessionError::StaleCredential`] when the password, status or existence of the user
    /// changed since `stamp` was taken.
    pub async fn create_client(
        &self,
        user_id: i64,
        stamp: &CredentialStamp,
        device: &ClientDevice,
        ip: Option<IpAddr>,
    ) -> Result<(Session, SessionToken), SessionError> {
        let (binding, device_info) = device.validate()?;
        let new = NewSession {
            user_id,
            kind: SessionKind::Client,
            device: Some(binding),
            device_info,
            ip,
        };
        self.create(new, stamp).await
    }

    /// Issues a console session (06 §3.2; the cookie is the admin API's business).
    pub async fn create_console(
        &self,
        user_id: i64,
        stamp: &CredentialStamp,
        ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<(Session, SessionToken), SessionError> {
        let new = NewSession {
            user_id,
            kind: SessionKind::Console,
            device: None,
            device_info: console_info(user_agent),
            ip,
        };
        self.create(new, stamp).await
    }

    /// Replaces a console session after a privilege change (a second factor was just
    /// enrolled through it): in one write transaction, checks that `session_id` is still a
    /// usable console session of an active user, revokes **every** console session of that
    /// user (this one included: no session that never passed the new factor keeps its
    /// standing) and issues a fresh one. Fails as [`SessionError::is_unauthenticated`] when
    /// the session no longer stands. Client sessions are left alone.
    pub async fn rotate_console(
        &self,
        session_id: i64,
        ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<(Session, SessionToken), SessionError> {
        let token = SessionToken::generate().map_err(SessionError::Random)?;
        let token_hash = token.hash().as_str().to_owned();
        let device_info = console_info(user_agent);
        let core = self.core.clone();
        let created = detached(async move {
            let result = core.rotate(session_id, token_hash, ip, device_info).await;
            if let Ok(created) = &result {
                // Commit-then-invalidate; a fill that read the old rows is dropped too.
                core.cache.invalidate_user(created.user_id);
            }
            result
        })
        .await?;
        Ok((created, token))
    }

    async fn create(
        &self,
        new: NewSession,
        stamp: &CredentialStamp,
    ) -> Result<(Session, SessionToken), SessionError> {
        let token = SessionToken::generate().map_err(SessionError::Random)?;
        let core = &self.core;
        let tx = begin_write(&core.db).await?;
        // Share-locks the user row (PostgreSQL) until commit: a concurrent password change
        // or disable waits, then revokes this session in its own transaction.
        if !self
            .users
            .credential_stamp_matches(&tx, new.user_id, stamp)
            .await?
        {
            return Err(SessionError::StaleCredential);
        }
        let row = core.new_row(new, token.hash().as_str().to_owned(), core.now_ms())?;
        let created = row.insert(&tx).await?;
        tx.commit().await?;
        Ok((created.into(), token))
    }
}

impl Core {
    async fn rotate(
        &self,
        session_id: i64,
        token_hash: String,
        ip: Option<IpAddr>,
        device_info: Option<String>,
    ) -> Result<Session, SessionError> {
        let tx = begin_write(&self.db).await?;
        let now = self.now_ms();
        let old = session::Entity::find_by_id(session_id)
            .lock_exclusive()
            .one(&tx)
            .await?
            .filter(|row| row.kind == SessionKind::Console)
            .ok_or(SessionError::InvalidToken)?;
        if old.revoked_at.is_some() {
            return Err(SessionError::Revoked);
        }
        let idle_deadline = old
            .last_used_at
            .timestamp_millis()
            .saturating_add(self.idle_ttl_ms());
        if now >= old.expires_at.timestamp_millis() || now >= idle_deadline {
            return Err(SessionError::Expired);
        }
        user::Entity::find_by_id(old.user_id)
            .lock_shared()
            .one(&tx)
            .await?
            .filter(|u| u.status == Status::Active)
            .ok_or(SessionError::UserInactive)?;
        let console = Condition::all()
            .add(session::Column::UserId.eq(old.user_id))
            .add(session::Column::Kind.eq(SessionKind::Console));
        revoke_rows(&tx, console, timestamp(now)?).await?;
        let new = NewSession {
            user_id: old.user_id,
            kind: SessionKind::Console,
            device: None,
            device_info,
            ip,
        };
        let created = self.new_row(new, token_hash, now)?.insert(&tx).await?;
        tx.commit().await?;
        Ok(created.into())
    }

    /// The row of a new session created `now`, expiring after the current session TTL.
    fn new_row(
        &self,
        new: NewSession,
        token_hash: String,
        now: i64,
    ) -> Result<session::ActiveModel, DbErr> {
        let (rustdesk_id, device_uuid) = match new.device {
            Some(d) => (Some(d.rustdesk_id), Some(d.uuid)),
            None => (None, None),
        };
        Ok(session::ActiveModel {
            token_hash: Set(token_hash),
            kind: Set(new.kind),
            user_id: Set(new.user_id),
            rustdesk_id: Set(rustdesk_id),
            device_uuid: Set(device_uuid),
            device_info: Set(new.device_info),
            ip: Set(new.ip.map(|ip| ip.to_string())),
            created_at: Set(timestamp(now)?),
            last_used_at: Set(timestamp(now)?),
            expires_at: Set(timestamp(now.saturating_add(self.session_ttl_ms()))?),
            revoked_at: Set(None),
            ..Default::default()
        })
    }
}
