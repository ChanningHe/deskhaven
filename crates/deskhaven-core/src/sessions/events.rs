use std::sync::Arc;

use tokio::sync::broadcast::{self, error::RecvError};
use tokio::task::JoinHandle;

use super::{Core, SessionService};
use crate::users::UserEvent;

impl SessionService {
    /// A password change revokes only sessions created before it; a lag flushes the whole cache.
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
