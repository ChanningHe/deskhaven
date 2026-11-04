//! "Test connection" and "test user" (the console's LDAP page, M3-09): the login steps one
//! by one, each with its own result, so an admin sees where a configuration breaks. A
//! failed step skips the ones after it. Messages name URLs, DNs and the directory's own
//! diagnostics, never a password; the typed test username is shown back only inside the
//! result. Nothing is written: no account is provisioned or synchronized. A diagnostic
//! holds one of the directory slots logins use and ends after a total timeout.

use std::collections::BTreeSet;

use deskhaven_db::entity::managed_by::ManagedBy;
use tokio::time::timeout;

use super::LdapService;
use super::directory::{DirectoryFault, Session};
use super::mapping;
use super::protocol::{self, DirectoryUser, Lookup};
use super::report::{
    CONNECTION_STEPS, Diagnostic, ProbedUser, Recorder, Step, StepStatus, USER_STEPS,
};
use super::settings::DirectorySettings;
use crate::users::Password;

impl LdapService {
    /// Connect and service bind, within the limits (see [`Self::test_user`]).
    pub async fn test_connection(&self, settings: &DirectorySettings) -> Diagnostic {
        let mut recorder = Recorder::new(CONNECTION_STEPS);
        let Some(_slot) = self.diagnostic_slot(settings, &mut recorder).await else {
            return recorder.finish();
        };
        let steps = self.connection_steps(settings, &mut recorder);
        if timeout(self.gate.limits().diagnostic_timeout, steps)
            .await
            .is_err()
        {
            recorder.time_out();
        }
        recorder.finish()
    }

    /// Every login step for `username`; the user bind only with a `password`. Writes
    /// nothing. Holds one of the directory slots logins use (a saturated directory fails the
    /// connect step as busy) and ends after [`DirectoryLimits::diagnostic_timeout`] in
    /// total, the steps not finished by then failed as timed out.
    ///
    /// [`DirectoryLimits::diagnostic_timeout`]: super::DirectoryLimits::diagnostic_timeout
    pub async fn test_user(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: Option<&Password>,
    ) -> Diagnostic {
        let mut recorder = Recorder::new(USER_STEPS);
        let Some(_slot) = self.diagnostic_slot(settings, &mut recorder).await else {
            return recorder.finish();
        };
        let steps = self.user_steps(settings, username, password, &mut recorder);
        if timeout(self.gate.limits().diagnostic_timeout, steps)
            .await
            .is_err()
        {
            recorder.time_out();
        }
        recorder.finish()
    }

    /// A directory slot, or the connect step failed as busy.
    async fn diagnostic_slot(
        &self,
        settings: &DirectorySettings,
        recorder: &mut Recorder,
    ) -> Option<tokio::sync::SemaphorePermit<'_>> {
        let slot = self.gate.slot().await;
        if slot.is_none() {
            tracing::warn!(directory = %settings.name, "directory checks saturated; diagnostic refused");
            let message = "the directory is busy (too many checks at once); try again";
            recorder.record(Step::Connect, StepStatus::Failed, message);
        }
        slot
    }

    async fn connection_steps(&self, settings: &DirectorySettings, recorder: &mut Recorder) {
        if let Some(mut session) = self.probe_connect(settings, recorder).await {
            probe_service_bind(session.as_mut(), settings, recorder).await;
            session.close().await;
        }
    }

    async fn user_steps(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: Option<&Password>,
        recorder: &mut Recorder,
    ) {
        if let Some(mut session) = self.probe_connect(settings, recorder).await {
            if probe_service_bind(session.as_mut(), settings, recorder).await {
                self.probe_user(session.as_mut(), settings, username, password, recorder)
                    .await;
            }
            session.close().await;
        }
    }

    async fn probe_connect(
        &self,
        settings: &DirectorySettings,
        recorder: &mut Recorder,
    ) -> Option<Box<dyn Session>> {
        let target = &settings.target;
        match self.inner.connector.connect(target).await {
            Ok(session) => {
                let tls = match (target.url.starts_with("ldaps:"), target.start_tls) {
                    (true, _) => "TLS",
                    (false, true) => "StartTLS",
                    (false, false) => "no TLS",
                };
                let verify = if target.tls_verify {
                    ""
                } else {
                    ", certificate NOT verified"
                };
                let message = format!("connected to {} ({tls}{verify})", target.url);
                recorder.record(Step::Connect, StepStatus::Passed, message);
                Some(session)
            }
            Err(fault) => {
                recorder.record(Step::Connect, StepStatus::Failed, fault.to_string());
                None
            }
        }
    }

    async fn probe_user(
        &self,
        session: &mut dyn Session,
        settings: &DirectorySettings,
        username: &str,
        password: Option<&Password>,
        recorder: &mut Recorder,
    ) {
        let password = password.filter(|p| !p.expose().is_empty());
        let probe = Probe {
            username,
            counted: password.is_some(),
        };
        let Some(user) = probe_search(session, settings, &probe, recorder).await else {
            return;
        };
        recorder.diagnostic.user = Some(probed(&user));
        if !probe_user_bind(session, &user, password, recorder).await {
            return;
        }
        // As a login would (an unmatched test name falls back to the first value).
        let name = user
            .username_matching(username)
            .or(user.usernames.first().map(String::as_str))
            .unwrap_or_default()
            .to_owned();
        match protocol::groups_of(session, settings, &user, &name).await {
            Ok(group_dns) => self.probe_mapping(settings, group_dns, recorder).await,
            Err(fault) => {
                recorder.record(Step::GroupMapping, StepStatus::Failed, fault.to_string())
            }
        }
    }

    async fn probe_mapping(
        &self,
        settings: &DirectorySettings,
        group_dns: Vec<String>,
        recorder: &mut Recorder,
    ) {
        let names = mapping::group_names(settings, &group_dns);
        let mapped = match &names {
            Some(names) => match self.mapped_groups(names).await {
                Ok(mapped) => mapped,
                Err(message) => {
                    return recorder.record(Step::GroupMapping, StepStatus::Failed, message);
                }
            },
            None => Vec::new(),
        };
        let admin = mapping::is_admin(settings, &group_dns);
        let message = mapping_message(group_dns.len(), names.is_some(), &mapped, admin);
        if let Some(user) = recorder.diagnostic.user.as_mut() {
            user.directory_groups = group_dns;
            user.mapped_groups = mapped;
            user.admin = admin;
        }
        recorder.record(Step::GroupMapping, StepStatus::Passed, message);
    }

    /// Names of the console-managed groups in `names` (lowercased), ordered by name.
    async fn mapped_groups(&self, names: &BTreeSet<String>) -> Result<Vec<String>, String> {
        let all = self.inner.groups.list().await.map_err(|err| {
            tracing::error!(error = %err, "cannot list groups for an LDAP diagnostic");
            "cannot read the deskhaven groups".to_owned()
        })?;
        Ok(all
            .into_iter()
            .filter(|g| g.managed_by == ManagedBy::Ui && names.contains(&g.name.to_lowercase()))
            .map(|g| g.name)
            .collect())
    }
}

/// `true` when the next steps may run.
async fn probe_service_bind(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    recorder: &mut Recorder,
) -> bool {
    let Some(service) = &settings.service else {
        recorder.record(
            Step::ServiceBind,
            StepStatus::Skipped,
            "no service account: searches run anonymously",
        );
        return true;
    };
    match protocol::bind_service(session, settings).await {
        Ok(()) => {
            recorder.record(
                Step::ServiceBind,
                StepStatus::Passed,
                format!("bound as {}", service.dn),
            );
            true
        }
        Err(DirectoryFault::InvalidCredentials) => {
            let message = format!(
                "the directory refused the bind DN {} or its password",
                service.dn
            );
            recorder.record(Step::ServiceBind, StepStatus::Failed, message);
            false
        }
        Err(fault) => {
            recorder.record(Step::ServiceBind, StepStatus::Failed, fault.to_string());
            false
        }
    }
}

/// The tested name, and whether a password came with it (a login would count it).
struct Probe<'a> {
    username: &'a str,
    counted: bool,
}

async fn probe_search(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    probe: &Probe<'_>,
    recorder: &mut Recorder,
) -> Option<DirectoryUser> {
    let found = protocol::find_user(session, settings, probe.username).await;
    // No single entry: a login refuses the name after a decoy bind, counted.
    if matches!(found, Ok(Lookup::Missing | Lookup::Ambiguous)) {
        recorder.diagnostic.attempted = probe.counted;
    }
    let (status, message, user) = match found {
        Ok(Lookup::Found(user)) => (StepStatus::Passed, format!("found {}", user.dn), Some(user)),
        Ok(Lookup::Missing) => (StepStatus::Failed, no_entry(settings), None),
        Ok(Lookup::Ambiguous) => (
            StepStatus::Failed,
            "several entries match: the username attribute must be unique".to_owned(),
            None,
        ),
        Err(fault) => (StepStatus::Failed, fault.to_string(), None),
    };
    recorder.record(Step::UserSearch, status, message);
    user
}

fn no_entry(settings: &DirectorySettings) -> String {
    format!(
        "no entry below {} matches {} with this {}",
        settings.user_base_dn, settings.user_filter, settings.username_attr
    )
}

/// `true` when the group step may run.
async fn probe_user_bind(
    session: &mut dyn Session,
    user: &DirectoryUser,
    password: Option<&Password>,
    recorder: &mut Recorder,
) -> bool {
    let Some(password) = password.filter(|p| !p.expose().is_empty()) else {
        recorder.record(Step::UserBind, StepStatus::Skipped, "no password given");
        return true;
    };
    // The directory sees the password from here on, whatever happens next.
    recorder.diagnostic.attempted = true;
    let (status, message) = match protocol::bind_user(session, &user.dn, password.expose()).await {
        Ok(true) => (
            StepStatus::Passed,
            "the directory accepted the password".to_owned(),
        ),
        Ok(false) => (
            StepStatus::Failed,
            "the directory refused the password".to_owned(),
        ),
        Err(fault) => (StepStatus::Failed, fault.to_string()),
    };
    recorder.record(Step::UserBind, status, message);
    status == StepStatus::Passed
}

fn probed(user: &DirectoryUser) -> ProbedUser {
    ProbedUser {
        dn: user.dn.clone(),
        username: user.usernames.first().cloned(),
        email: user.email.clone(),
        display_name: user.display_name.clone(),
        directory_groups: Vec::new(),
        mapped_groups: Vec::new(),
        admin: None,
    }
}

fn mapping_message(found: usize, mapping: bool, mapped: &[String], admin: Option<bool>) -> String {
    let groups = match (mapping, mapped.is_empty()) {
        (false, _) => "group mapping off (no group base DN)".to_owned(),
        (true, true) => "no deskhaven group matches".to_owned(),
        (true, false) => format!("member of {}", mapped.join(", ")),
    };
    let admin = match admin {
        None => "no admin group",
        Some(true) => "admin",
        Some(false) => "not admin",
    };
    format!("{found} directory group(s); {groups}; {admin}")
}
