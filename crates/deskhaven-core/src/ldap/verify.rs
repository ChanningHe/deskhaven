//! A stalled or flooded directory costs logins a bounded wait and never piles up tasks and
//! connections. Once a password bind has been sent, any failure is `DirectoryInterrupted`: the
//! directory has seen the password, so the login throttle counts the attempt.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::{Semaphore, SemaphorePermit};
use tokio::time::timeout;

use super::LdapService;
use super::directory::{DirectoryFault, Session};
use super::protocol::{self, DirectoryUser, Lookup};
use super::settings::DirectorySettings;
use crate::users::AuthError;

/// Password checks and the console's diagnostics share the slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryLimits {
    pub concurrent: usize,
    pub queue_timeout: Duration,
    pub login_timeout: Duration,
    pub diagnostic_timeout: Duration,
}

impl Default for DirectoryLimits {
    fn default() -> Self {
        Self {
            concurrent: 32,
            queue_timeout: Duration::from_secs(5),
            login_timeout: Duration::from_secs(15),
            diagnostic_timeout: Duration::from_secs(20),
        }
    }
}

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

    pub(crate) fn limits(&self) -> DirectoryLimits {
        self.limits
    }

    pub(crate) async fn slot(&self) -> Option<SemaphorePermit<'_>> {
        match timeout(self.limits.queue_timeout, self.permits.acquire()).await {
            Ok(Ok(permit)) => Some(permit),
            _ => None,
        }
    }
}

pub(crate) struct VerifiedUser {
    pub(crate) user: DirectoryUser,
    pub(crate) name: String,
    pub(crate) group_dns: Vec<String>,
}

impl LdapService {
    pub(crate) async fn verify(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: &str,
    ) -> Result<Option<VerifiedUser>, AuthError> {
        let gate = &self.gate;
        let Some(_slot) = gate.slot().await else {
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
