//! Step 3: the client's polling (see the module docs of `oidc`).

use deskhaven_db::entity::oidc_flow::{self, Column, Flow};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::callback::OidcLogin;
use super::error::{Failure, PollError};
use super::flow::timestamp;
use super::{OidcService, context};
use crate::sessions::{ClientDevice, TokenHash};
use crate::users::UserError;

/// What a poll finds.
#[derive(Debug)]
pub enum PollOutcome {
    /// The callback has not completed yet.
    Pending,
    /// The flow failed; the same answer until it expires.
    Failed(Failure),
    /// The login, handed out once.
    Ready(Box<ClientLogin>),
}

/// A finished client login: mint the client session for `device` under `login.stamp`.
#[derive(Debug)]
pub struct ClientLogin {
    pub login: OidcLogin,
    /// The device the flow was started from (and polled from).
    pub device: ClientDevice,
}

impl OidcService {
    /// The state of the client flow of `code`, polled from the device `rustdesk_id` /
    /// `uuid` (it must be the one the flow was started from).
    pub async fn poll_client(
        &self,
        code: &str,
        rustdesk_id: &str,
        uuid: &str,
    ) -> Result<PollOutcome, PollError> {
        let hash = TokenHash::of_presented(code).ok_or(PollError::Invalid)?;
        let now = timestamp(self.inner.now_ms())?;
        let row = oidc_flow::Entity::find()
            .filter(Column::ClientCodeHash.eq(hash.as_str()))
            .filter(Column::Flow.eq(Flow::Client))
            .filter(Column::ExpiresAt.gt(now))
            .one(&self.inner.db)
            .await?
            .ok_or(PollError::Invalid)?;
        let device = context::client_device(&row.context).ok_or_else(|| corrupt(row.id))?;
        if device.rustdesk_id != rustdesk_id.trim() || device.uuid != uuid.trim() {
            return Err(PollError::Invalid);
        }
        if let Some(code) = &row.error {
            return Ok(PollOutcome::Failed(
                Failure::from_code(code).unwrap_or(Failure::Unavailable),
            ));
        }
        let Some(user_id) = row.completed_user_id else {
            return Ok(PollOutcome::Pending);
        };
        let stamp = context::stamp(&row.context).ok_or_else(|| corrupt(row.id))?;
        // Single hand-out: whoever deletes the row owns the login.
        let taken = oidc_flow::Entity::delete_many()
            .filter(Column::Id.eq(row.id))
            .filter(Column::CompletedUserId.is_not_null())
            .exec(&self.inner.db)
            .await?;
        if taken.rows_affected != 1 {
            return Err(PollError::Invalid);
        }
        let user = match self.inner.users.get(user_id).await {
            Ok(user) => user,
            Err(UserError::NotFound) => return Err(PollError::Invalid),
            Err(err) => return Err(PollError::Internal(err.into())),
        };
        Ok(PollOutcome::Ready(Box::new(ClientLogin {
            login: OidcLogin { user, stamp },
            device,
        })))
    }
}

fn corrupt(flow_id: i64) -> PollError {
    tracing::error!(flow_id, "OIDC flow context is unreadable");
    PollError::Internal("corrupt OIDC flow context".into())
}
