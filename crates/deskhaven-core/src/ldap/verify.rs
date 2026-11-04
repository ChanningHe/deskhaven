//! The directory part of a password check, bounded: at most
//! [`DirectoryLimits::concurrent`] at once (a slot is waited for at most
//! [`DirectoryLimits::queue_timeout`]), each within [`DirectoryLimits::login_timeout`] in
//! total, connect to close. A stalled or flooded directory therefore costs logins a bounded
//! wait and never piles up tasks and connections; both limits answer
//! [`AuthError::DirectoryUnavailable`]. Once a password bind has been sent (the user's or
//! the decoy's), any failure or timeout is [`AuthError::DirectoryInterrupted`] instead:
//! the directory has seen the password, so the login throttle counts the attempt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::time::timeout;

use super::LdapService;
use super::directory::{DirectoryFault, Session};
use super::protocol::{self, DirectoryUser, Lookup};
use super::settings::DirectorySettings;
use crate::users::AuthError;

/// Bounds on directory work during password checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryLimits {
    /// Directory checks running at once.
    pub concurrent: usize,
    /// Longest wait for a free slot.
    pub queue_timeout: Duration,
    /// Longest whole check (connect, binds, searches, close).
    pub login_timeout: Duration,
}

impl Default for DirectoryLimits {
    fn default() -> Self {
        Self {
            concurrent: 32,
            queue_timeout: Duration::from_secs(5),
            login_timeout: Duration::from_secs(15),
        }
    }
}

/// The slots of [`DirectoryLimits::concurrent`].
pub(crate) struct Gate {
    permits: Semaphore,
    limits: DirectoryLimits,
}

impl Gate {
    pub(crate) fn new(limits: DirectoryLimits) -> Self {
        Self {
            permits: Semaphore::new(limits.concurrent),
            limits,
        }
    }
}

/// A directory user whose password the directory accepted.
pub(crate) struct VerifiedUser {
    pub(crate) user: DirectoryUser,
    /// The username attribute value that is the typed username.
    pub(crate) name: String,
    pub(crate) group_dns: Vec<String>,
}

impl LdapService {
    /// The directory steps for `username`, within the limits. `None`: the directory refused
    /// the login.
    pub(crate) async fn verify(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: &str,
    ) -> Result<Option<VerifiedUser>, AuthError> {
        let gate = &self.gate;
        let Ok(Ok(_slot)) = timeout(gate.limits.queue_timeout, gate.permits.acquire()).await else {
            tracing::warn!(directory = %settings.name, "directory logins saturated; refused");
            return Err(AuthError::DirectoryUnavailable);
        };
        let sent = AtomicBool::new(false);
        let checked = self.connected_steps(settings, username, password, &sent);
        let outcome = match timeout(gate.limits.login_timeout, checked).await {
            Ok(outcome) => outcome,
            Err(_) => {
                tracing::warn!(directory = %settings.name, "directory login timed out");
                Err(AuthError::DirectoryUnavailable)
            }
        };
        match outcome {
            // The directory has seen the password: an attempt, whatever happened next.
            Err(AuthError::DirectoryUnavailable) if sent.load(Ordering::Acquire) => {
                Err(AuthError::DirectoryInterrupted)
            }
            outcome => outcome,
        }
    }

    async fn connected_steps(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: &str,
        sent: &AtomicBool,
    ) -> Result<Option<VerifiedUser>, AuthError> {
        let mut session = self
            .inner
            .connector
            .connect(&settings.target)
            .await
            .map_err(|fault| unavailable(settings, "connect", &fault))?;
        let verified = steps(session.as_mut(), settings, username, password, sent).await;
        session.close().await;
        verified
    }
}

async fn steps(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    username: &str,
    password: &str,
    sent: &AtomicBool,
) -> Result<Option<VerifiedUser>, AuthError> {
    let fail = |step| move |fault: DirectoryFault| unavailable(settings, step, &fault);
    protocol::bind_service(session, settings)
        .await
        .map_err(fail("service bind"))?;
    let found = protocol::find_user(session, settings, username)
        .await
        .map_err(fail("user search"))?;
    let user = match found {
        Lookup::Found(user) => user,
        Lookup::Missing => {
            sent.store(true, Ordering::Release);
            protocol::decoy_bind(session, settings, password)
                .await
                .map_err(fail("user bind"))?;
            return Ok(None);
        }
        Lookup::Ambiguous => {
            tracing::warn!(directory = %settings.name, "several directory entries match one username; login refused");
            return Ok(None);
        }
    };
    sent.store(true, Ordering::Release);
    let accepted = protocol::bind_user(session, &user.dn, password)
        .await
        .map_err(fail("user bind"))?;
    if !accepted {
        return Ok(None);
    }
    let Some(name) = user.username_matching(username).map(str::to_owned) else {
        tracing::warn!(directory = %settings.name, "directory username attribute does not match the login name; refused");
        return Ok(None);
    };
    let group_dns = protocol::groups_of(session, settings, &user, &name)
        .await
        .map_err(fail("group lookup"))?;
    Ok(Some(VerifiedUser {
        user,
        name,
        group_dns,
    }))
}

fn unavailable(
    settings: &DirectorySettings,
    step: &'static str,
    fault: &DirectoryFault,
) -> AuthError {
    tracing::warn!(directory = %settings.name, step, error = %fault, "directory login failed: directory unavailable or misconfigured");
    AuthError::DirectoryUnavailable
}
