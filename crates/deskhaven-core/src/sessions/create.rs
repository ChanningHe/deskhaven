//! Session creation: one write transaction that re-checks the login's credential stamp and
//! inserts the hashed token.

use std::net::IpAddr;

use deskhaven_db::begin_write;
use deskhaven_db::entity::session;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;

use super::model::console_info;
use super::{
    ClientDevice, DeviceBinding, Session, SessionError, SessionKind, SessionService, SessionToken,
    timestamp,
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
        let now = core.now_ms();
        let (rustdesk_id, device_uuid) = match new.device {
            Some(d) => (Some(d.rustdesk_id), Some(d.uuid)),
            None => (None, None),
        };
        let row = session::ActiveModel {
            token_hash: Set(token.hash().as_str().to_owned()),
            kind: Set(new.kind),
            user_id: Set(new.user_id),
            rustdesk_id: Set(rustdesk_id),
            device_uuid: Set(device_uuid),
            device_info: Set(new.device_info),
            ip: Set(new.ip.map(|ip| ip.to_string())),
            created_at: Set(timestamp(now)?),
            last_used_at: Set(timestamp(now)?),
            expires_at: Set(timestamp(now.saturating_add(core.session_ttl_ms()))?),
            revoked_at: Set(None),
            ..Default::default()
        };
        let created = row.insert(&tx).await?;
        tx.commit().await?;
        Ok((created.into(), token))
    }
}
