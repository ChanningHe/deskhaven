//! The steps of a directory login, shared by logins and diagnostics: service bind, user
//! search, user bind, group lookup. Passwords are only ever handed to [`Session::bind`];
//! an empty one never is (RFC 4513 §5.1.2 unauthenticated bind).

use super::directory::{DirectoryFault, Entry, Scope, SearchRequest, Session};
use super::filter;
use super::settings::{DirectorySettings, MEMBER_OF};
use crate::crypto::fill_random;

/// Group DNs beyond this many are ignored.
pub(crate) const MAX_GROUPS: usize = 256;
/// The user search wants exactly one entry; more matches are ambiguous (`truncated`).
const USER_SEARCH_LIMIT: usize = 1;
/// Random bytes of the decoy DN's RDN value.
const DECOY_BYTES: usize = 12;

/// The directory entry of a username.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectoryUser {
    pub(crate) dn: String,
    /// Values of the username attribute.
    pub(crate) usernames: Vec<String>,
    pub(crate) email: Option<String>,
    pub(crate) display_name: Option<String>,
    /// `memberOf` values (when asked for).
    member_of: Vec<String>,
}

impl DirectoryUser {
    fn of(entry: Entry, settings: &DirectorySettings) -> Self {
        let first = |attr: &Option<String>| {
            attr.as_deref()
                .and_then(|a| entry.first(a))
                .map(str::to_owned)
        };
        Self {
            usernames: entry.values(&settings.username_attr).to_vec(),
            email: first(&settings.email_attr),
            display_name: first(&settings.display_name_attr),
            member_of: entry
                .values(MEMBER_OF)
                .iter()
                .take(MAX_GROUPS)
                .cloned()
                .collect(),
            dn: entry.dn,
        }
    }

    /// The value of the username attribute that is `typed` (same normalized username).
    pub(crate) fn username_matching(&self, typed: &str) -> Option<&str> {
        let wanted = crate::users::username_norm(typed)?;
        self.usernames
            .iter()
            .map(String::as_str)
            .find(|v| crate::users::username_norm(v).as_deref() == Some(wanted.as_str()))
    }
}

pub(crate) enum Lookup {
    Found(DirectoryUser),
    Missing,
    /// More than one entry matched: refused, the username is not unique in the directory.
    Ambiguous,
}

/// Binds as the service account; an anonymous directory has nothing to do. A service
/// account without a stored password is refused here, never sent as an empty bind.
pub(crate) async fn bind_service(
    session: &mut dyn Session,
    settings: &DirectorySettings,
) -> Result<(), DirectoryFault> {
    let Some(service) = &settings.service else {
        return Ok(());
    };
    match service.password.as_deref().filter(|p| !p.is_empty()) {
        Some(password) => session.bind(&service.dn, password).await,
        None => Err(DirectoryFault::InvalidCredentials),
    }
}

pub(crate) async fn find_user(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    username: &str,
) -> Result<Lookup, DirectoryFault> {
    let request = SearchRequest {
        base: settings.user_base_dn.clone(),
        scope: Scope::Subtree,
        filter: filter::user_search(&settings.user_filter, &settings.username_attr, username),
        attrs: settings.user_attrs(),
        size_limit: USER_SEARCH_LIMIT,
    };
    let found = session.search(&request).await?;
    let mut entries = found.entries.into_iter();
    match (entries.next(), entries.next(), found.truncated) {
        (None, _, false) => Ok(Lookup::Missing),
        (Some(entry), None, false) => Ok(Lookup::Found(DirectoryUser::of(entry, settings))),
        _ => Ok(Lookup::Ambiguous),
    }
}

/// `Ok(false)` for a refused bind (wrong password, or the directory refusing the account,
/// e.g. locked); `Err` only when the directory could not answer.
pub(crate) async fn bind_user(
    session: &mut dyn Session,
    dn: &str,
    password: &str,
) -> Result<bool, DirectoryFault> {
    if password.is_empty() {
        return Ok(false);
    }
    match session.bind(dn, password).await {
        Ok(()) => Ok(true),
        Err(DirectoryFault::InvalidCredentials) => Ok(false),
        Err(DirectoryFault::Refused { code, .. }) => {
            tracing::debug!(code, "the directory refused a user bind");
            Ok(false)
        }
        Err(fault) => Err(fault),
    }
}

/// A bind for a username the directory does not know, so that an unknown username costs
/// the same round trips as a wrong password: a random DN below the user base, which cannot
/// succeed. Only an outage is an error.
pub(crate) async fn decoy_bind(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    password: &str,
) -> Result<(), DirectoryFault> {
    let mut random = [0u8; DECOY_BYTES];
    if fill_random(&mut random).is_err() {
        return Ok(());
    }
    let value: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let dn = format!("cn=deskhaven-decoy-{value},{}", settings.user_base_dn);
    bind_user(session, &dn, password).await.map(|_| ())
}

/// The user's group DNs: a group search as the service account (after the user's own
/// bind) when a group filter is set, else the entry's `memberOf`. Empty when neither group
/// mapping nor the admin flag needs them.
pub(crate) async fn groups_of(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    user: &DirectoryUser,
) -> Result<Vec<String>, DirectoryFault> {
    if !settings.wants_groups() {
        return Ok(Vec::new());
    }
    let Some(base) = settings
        .group_base_dn
        .as_ref()
        .filter(|_| settings.searches_groups())
    else {
        return Ok(user.member_of.clone());
    };
    bind_service(session, settings).await?;
    let username = user.usernames.first().map_or("", String::as_str);
    let request = SearchRequest {
        base: base.clone(),
        scope: Scope::Subtree,
        filter: filter::group_search(settings.group_template(), &user.dn, username),
        attrs: Vec::new(),
        size_limit: MAX_GROUPS,
    };
    let found = session.search(&request).await?;
    if found.truncated {
        tracing::warn!(directory = %settings.name, "group search truncated at {MAX_GROUPS} groups");
    }
    Ok(found.entries.into_iter().map(|e| e.dn).collect())
}
