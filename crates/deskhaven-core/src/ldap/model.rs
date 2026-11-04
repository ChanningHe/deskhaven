//! Directory types and input validation (04 §3 `ldap_directories`, 06 §3.5).

use std::fmt;

use deskhaven_db::entity::ldap_directory;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::time::TimestampMs;

use super::error::DirectoryError;
use super::validate;

/// Default `user_filter`.
pub const DEFAULT_USER_FILTER: &str = "(objectClass=person)";
/// Default `username_attr`.
pub const DEFAULT_USERNAME_ATTR: &str = "uid";

/// A directory as seen outside the service: never the bind password, only whether one is
/// set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Directory {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub start_tls: bool,
    /// `false` only when set explicitly (the console shows a warning).
    pub tls_verify: bool,
    /// `None`: searches run anonymously.
    pub bind_dn: Option<String>,
    pub has_bind_password: bool,
    pub user_base_dn: String,
    pub user_filter: String,
    pub username_attr: String,
    pub email_attr: Option<String>,
    pub display_name_attr: Option<String>,
    /// Set: group mapping is on, for groups below this DN.
    pub group_base_dn: Option<String>,
    /// Set: groups come from this search (placeholders `{user_dn}`, `{username}`); else
    /// from the user entry's `memberOf`.
    pub group_filter: Option<String>,
    /// Members of this group are admins (owners stay owners).
    pub admin_group_dn: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<ldap_directory::Model> for Directory {
    fn from(m: ldap_directory::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            url: m.url,
            start_tls: m.start_tls,
            tls_verify: m.tls_verify,
            bind_dn: m.bind_dn,
            has_bind_password: m.bind_password_enc.is_some(),
            user_base_dn: m.user_base_dn,
            user_filter: m.user_filter,
            username_attr: m.username_attr,
            email_attr: m.email_attr,
            display_name_attr: m.display_name_attr,
            group_base_dn: m.group_base_dn,
            group_filter: m.group_filter,
            admin_group_dn: m.admin_group_dn,
            enabled: m.enabled,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Input of [`LdapService::create_directory`](super::LdapService::create_directory).
/// `Debug` never shows the bind password.
#[derive(Clone)]
pub struct NewDirectory {
    pub name: String,
    /// `ldaps://host[:port]`, or `ldap://` with `start_tls` (plain only on loopback).
    pub url: String,
    pub start_tls: bool,
    pub tls_verify: bool,
    pub bind_dn: Option<String>,
    /// Required with `bind_dn`.
    pub bind_password: Option<String>,
    pub user_base_dn: String,
    /// Blank: [`DEFAULT_USER_FILTER`].
    pub user_filter: String,
    /// Blank: [`DEFAULT_USERNAME_ATTR`].
    pub username_attr: String,
    pub email_attr: Option<String>,
    pub display_name_attr: Option<String>,
    pub group_base_dn: Option<String>,
    pub group_filter: Option<String>,
    pub admin_group_dn: Option<String>,
    pub enabled: bool,
}

impl NewDirectory {
    /// An enabled, anonymous, certificate-verifying directory with the default filter and
    /// username attribute, no group mapping.
    pub fn new(
        name: impl Into<String>,
        url: impl Into<String>,
        user_base_dn: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            start_tls: false,
            tls_verify: true,
            bind_dn: None,
            bind_password: None,
            user_base_dn: user_base_dn.into(),
            user_filter: DEFAULT_USER_FILTER.to_owned(),
            username_attr: DEFAULT_USERNAME_ATTR.to_owned(),
            email_attr: None,
            display_name_attr: None,
            group_base_dn: None,
            group_filter: None,
            admin_group_dn: None,
            enabled: true,
        }
    }
}

impl fmt::Debug for NewDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewDirectory")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("bind_dn", &self.bind_dn)
            .field("has_bind_password", &self.bind_password.is_some())
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// Fields to change; `None` leaves a field as it is, `Some(None)` clears an optional one.
/// `Debug` never shows the bind password.
#[derive(Clone, Default)]
pub struct DirectoryUpdate {
    pub name: Option<String>,
    pub url: Option<String>,
    pub start_tls: Option<bool>,
    pub tls_verify: Option<bool>,
    pub bind_dn: Option<Option<String>>,
    pub bind_password: Option<Option<String>>,
    pub user_base_dn: Option<String>,
    pub user_filter: Option<String>,
    pub username_attr: Option<String>,
    pub email_attr: Option<Option<String>>,
    pub display_name_attr: Option<Option<String>>,
    pub group_base_dn: Option<Option<String>>,
    pub group_filter: Option<Option<String>>,
    pub admin_group_dn: Option<Option<String>>,
    pub enabled: Option<bool>,
}

impl fmt::Debug for DirectoryUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DirectoryUpdate")
            .field("name", &self.name)
            .field("url", &self.url)
            .field("changes_bind_password", &self.bind_password.is_some())
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// The validated, non-secret columns of a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fields {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) start_tls: bool,
    pub(crate) tls_verify: bool,
    pub(crate) bind_dn: Option<String>,
    pub(crate) user_base_dn: String,
    pub(crate) user_filter: String,
    pub(crate) username_attr: String,
    pub(crate) email_attr: Option<String>,
    pub(crate) display_name_attr: Option<String>,
    pub(crate) group_base_dn: Option<String>,
    pub(crate) group_filter: Option<String>,
    pub(crate) admin_group_dn: Option<String>,
    pub(crate) enabled: bool,
}

impl Fields {
    pub(crate) fn of_new(new: &NewDirectory) -> Result<Self, DirectoryError> {
        let fields = Self {
            name: validate::name(&new.name)?,
            url: new.url.trim().to_owned(),
            start_tls: new.start_tls,
            tls_verify: new.tls_verify,
            bind_dn: validate::optional_dn("bind_dn", new.bind_dn.as_deref())?,
            user_base_dn: validate::dn("user_base_dn", &new.user_base_dn)?,
            user_filter: validate::user_filter(&new.user_filter)?,
            username_attr: validate::username_attr(&new.username_attr)?,
            email_attr: validate::optional_attr("email_attr", new.email_attr.as_deref())?,
            display_name_attr: validate::optional_attr(
                "display_name_attr",
                new.display_name_attr.as_deref(),
            )?,
            group_base_dn: validate::optional_dn("group_base_dn", new.group_base_dn.as_deref())?,
            group_filter: validate::group_filter(new.group_filter.as_deref())?,
            admin_group_dn: validate::optional_dn("admin_group_dn", new.admin_group_dn.as_deref())?,
            enabled: new.enabled,
        };
        fields.check()?;
        Ok(fields)
    }

    /// `current` with `update` applied (the bind password is the caller's).
    pub(crate) fn updated(
        current: &ldap_directory::Model,
        update: &DirectoryUpdate,
    ) -> Result<Self, DirectoryError> {
        let fields = Self {
            name: pick(&update.name, &current.name, validate::name)?,
            url: update
                .url
                .as_deref()
                .map_or(&current.url[..], str::trim)
                .to_owned(),
            start_tls: update.start_tls.unwrap_or(current.start_tls),
            tls_verify: update.tls_verify.unwrap_or(current.tls_verify),
            bind_dn: pick_opt(&update.bind_dn, &current.bind_dn, |v| {
                validate::optional_dn("bind_dn", v)
            })?,
            user_base_dn: pick(&update.user_base_dn, &current.user_base_dn, |v| {
                validate::dn("user_base_dn", v)
            })?,
            user_filter: pick(
                &update.user_filter,
                &current.user_filter,
                validate::user_filter,
            )?,
            username_attr: pick(
                &update.username_attr,
                &current.username_attr,
                validate::username_attr,
            )?,
            email_attr: pick_opt(&update.email_attr, &current.email_attr, |v| {
                validate::optional_attr("email_attr", v)
            })?,
            display_name_attr: pick_opt(
                &update.display_name_attr,
                &current.display_name_attr,
                |v| validate::optional_attr("display_name_attr", v),
            )?,
            group_base_dn: pick_opt(&update.group_base_dn, &current.group_base_dn, |v| {
                validate::optional_dn("group_base_dn", v)
            })?,
            group_filter: pick_opt(
                &update.group_filter,
                &current.group_filter,
                validate::group_filter,
            )?,
            admin_group_dn: pick_opt(&update.admin_group_dn, &current.admin_group_dn, |v| {
                validate::optional_dn("admin_group_dn", v)
            })?,
            enabled: update.enabled.unwrap_or(current.enabled),
        };
        fields.check()?;
        Ok(fields)
    }

    /// Rules across fields (the bind password's are the caller's).
    fn check(&self) -> Result<(), DirectoryError> {
        validate::url(&self.url, self.start_tls)?;
        if self.group_filter.is_some() && self.group_base_dn.is_none() {
            return Err(validate::invalid(
                "group_base_dn",
                "is required with a group filter",
            ));
        }
        Ok(())
    }
}

fn pick<F>(new: &Option<String>, current: &str, check: F) -> Result<String, DirectoryError>
where
    F: Fn(&str) -> Result<String, DirectoryError>,
{
    match new {
        Some(raw) => check(raw),
        None => Ok(current.to_owned()),
    }
}

fn pick_opt<F>(
    new: &Option<Option<String>>,
    current: &Option<String>,
    check: F,
) -> Result<Option<String>, DirectoryError>
where
    F: Fn(Option<&str>) -> Result<Option<String>, DirectoryError>,
{
    match new {
        Some(raw) => check(raw.as_deref()),
        None => Ok(current.clone()),
    }
}
