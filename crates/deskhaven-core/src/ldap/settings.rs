//! A directory as the protocol code uses it: validated fields and the opened bind password.

use std::fmt;

use deskhaven_db::entity::ldap_directory;
use zeroize::Zeroizing;

use super::directory::Target;
use super::error::DirectoryError;
use super::filter::DEFAULT_GROUP_FILTER;
use super::model::{Fields, NewDirectory};
use super::validate;

/// Attribute holding the groups of a user entry (AD, OpenLDAP's memberof overlay).
pub(crate) const MEMBER_OF: &str = "memberOf";

/// The service account searches run as. The password is `None` only when the stored row
/// lost it (validation never allows that); binding then fails as a configuration error.
#[derive(Clone)]
pub(crate) struct ServiceAccount {
    pub(crate) dn: String,
    pub(crate) password: Option<Zeroizing<String>>,
}

/// Everything needed to talk to one directory. `Debug` never shows the bind password.
#[derive(Clone)]
pub struct DirectorySettings {
    pub(crate) name: String,
    pub(crate) target: Target,
    pub(crate) service: Option<ServiceAccount>,
    pub(crate) user_base_dn: String,
    pub(crate) user_filter: String,
    pub(crate) username_attr: String,
    pub(crate) email_attr: Option<String>,
    pub(crate) display_name_attr: Option<String>,
    pub(crate) group_base_dn: Option<String>,
    pub(crate) group_filter: Option<String>,
    pub(crate) admin_group_dn: Option<String>,
}

impl DirectorySettings {
    /// Settings of a directory that is not stored (yet), e.g. to test a form before saving.
    pub fn from_new(new: &NewDirectory) -> Result<Self, DirectoryError> {
        let fields = Fields::of_new(new)?;
        let password = validate::bind_password(new.bind_password.as_deref())?;
        validate::bind_pair(fields.bind_dn.is_some(), password.is_some())?;
        Ok(Self::of(fields, password.map(Zeroizing::new)))
    }

    pub(crate) fn from_row(
        row: &ldap_directory::Model,
        password: Option<Zeroizing<String>>,
    ) -> Self {
        let fields = Fields {
            name: row.name.clone(),
            url: row.url.clone(),
            start_tls: row.start_tls,
            tls_verify: row.tls_verify,
            bind_dn: row.bind_dn.clone(),
            user_base_dn: row.user_base_dn.clone(),
            user_filter: row.user_filter.clone(),
            username_attr: row.username_attr.clone(),
            email_attr: row.email_attr.clone(),
            display_name_attr: row.display_name_attr.clone(),
            group_base_dn: row.group_base_dn.clone(),
            group_filter: row.group_filter.clone(),
            admin_group_dn: row.admin_group_dn.clone(),
            enabled: row.enabled,
        };
        Self::of(fields, password)
    }

    fn of(fields: Fields, password: Option<Zeroizing<String>>) -> Self {
        Self {
            name: fields.name,
            target: Target {
                url: fields.url,
                start_tls: fields.start_tls,
                tls_verify: fields.tls_verify,
            },
            service: fields.bind_dn.map(|dn| ServiceAccount { dn, password }),
            user_base_dn: fields.user_base_dn,
            user_filter: fields.user_filter,
            username_attr: fields.username_attr,
            email_attr: fields.email_attr,
            display_name_attr: fields.display_name_attr,
            group_base_dn: fields.group_base_dn,
            group_filter: fields.group_filter,
            admin_group_dn: fields.admin_group_dn,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether groups come from a group search (else from `memberOf`).
    pub(crate) fn searches_groups(&self) -> bool {
        self.group_filter.is_some() && self.group_base_dn.is_some()
    }

    /// Whether group DNs are needed at all (mapping or the admin flag).
    pub(crate) fn wants_groups(&self) -> bool {
        self.group_base_dn.is_some() || self.admin_group_dn.is_some()
    }

    pub(crate) fn group_template(&self) -> &str {
        self.group_filter.as_deref().unwrap_or(DEFAULT_GROUP_FILTER)
    }

    /// Attributes the user search asks for.
    pub(crate) fn user_attrs(&self) -> Vec<String> {
        let mut attrs = vec![self.username_attr.clone()];
        attrs.extend(self.email_attr.iter().cloned());
        attrs.extend(self.display_name_attr.iter().cloned());
        if self.wants_groups() && !self.searches_groups() {
            attrs.push(MEMBER_OF.to_owned());
        }
        attrs
    }
}

impl fmt::Debug for DirectorySettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let service = self.service.as_ref().map(|s| &s.dn);
        f.debug_struct("DirectorySettings")
            .field("name", &self.name)
            .field("target", &self.target)
            .field("service_dn", &service)
            .field("user_base_dn", &self.user_base_dn)
            .field("user_filter", &self.user_filter)
            .field("group_base_dn", &self.group_base_dn)
            .finish_non_exhaustive()
    }
}
