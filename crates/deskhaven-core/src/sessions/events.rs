//! Consumer of [`UserEvent`]s (contract in `users::events`).

use std::sync::Arc;

use tokio::sync::broadcast::{self, error::RecvError};
use tokio::task::JoinHandle;

use super::{Core, SessionService};
use crate::users::UserEvent;

impl SessionService {
    /// Spawns a task that applies `events` to this service until the channel closes:
    /// every [`UserEvent`] revokes the sessions of its user (for a password change, only
    /// those created before it) and drops them from the cache;
    /// a lag (lost events) flushes the whole cache. [`SessionService::new`] already runs
    /// one for its `UserService`; this is for further sources and for tests.
    pub fn spawn_event_consumer(&self, events: broadcast::Receiver<UserEvent>) -> JoinHandle<()> {
        tokio::spawn(consume(Arc::clone(&self.core), events))
    }
}

async fn consume(core: Arc<Core>, mut events: broadcast::Receiver<UserEvent>) {
    loop {
        match events.recv().await {
            Ok(event) => {
                let user_id = event.user_id();
                let revoked = match event {
                    UserEvent::PasswordChanged { .. } => {
                        core.revoke_sessions_before_password_change(user_id).await
                    }
                    _ => core.revoke_all_for_user(user_id).await,
                };
                if let Err(err) = revoked {
                    // The cache entries are gone regardless; read-time checks and the
                    // in-transaction revocation of the account change still apply.
                    tracing::error!(user_id, ?event, error = %err, "revoking sessions after a user event failed");
                }
            }
            Err(RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "user events lost; flushing the session cache");
                core.cache.flush();
            }
            Err(RecvError::Closed) => return,
        }
    }
}
