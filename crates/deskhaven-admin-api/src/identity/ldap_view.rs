//! Wire types of the LDAP directory endpoints. The bind password is write-only: requests
//! may carry it, responses only say whether one is set. Timestamps are unix milliseconds.

use deskhaven_core::ldap::{Directory, DirectoryUpdate, NewDirectory};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::extract::present;
use crate::groups::ManagedByView;

const MAX_NAME: usize = 64;
const MAX_URL: usize = 2048;
const MAX_DN: usize = 1024;
const MAX_FILTER: usize = 1024;
const MAX_ATTR: usize = 64;
const MAX_BIND_PASSWORD: usize = 1024;

/// A directory, without its bind password.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LdapDirectoryView {
    pub id: i64,
    pub name: String,
    /// `ldaps://host[:port]`, or `ldap://` with StartTLS (plain only to a loopback host).
    pub url: String,
    pub start_tls: bool,
    /// `false`: the server certificate is NOT verified (shown as a warning).
    pub tls_verify: bool,
    /// `null`: searches run anonymously.
    pub bind_dn: Option<String>,
    pub has_bind_password: bool,
    pub user_base_dn: String,
    pub user_filter: String,
    pub username_attr: String,
    pub email_attr: Option<String>,
    pub display_name_attr: Option<String>,
    /// Set: group mapping is on, for groups below this DN.
    pub group_base_dn: Option<String>,
    /// Set: groups come from this search (`{user_dn}`, `{username}`); else from `memberOf`.
    pub group_filter: Option<String>,
    /// Members of this group are admins (owners stay owners).
    pub admin_group_dn: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedByView,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Directory> for LdapDirectoryView {
    fn from(d: Directory) -> Self {
        Self {
            id: d.id,
            name: d.name,
            url: d.url,
            start_tls: d.start_tls,
            tls_verify: d.tls_verify,
            bind_dn: d.bind_dn,
            has_bind_password: d.has_bind_password,
            user_base_dn: d.user_base_dn,
            user_filter: d.user_filter,
            username_attr: d.username_attr,
            email_attr: d.email_attr,
            display_name_attr: d.display_name_attr,
            group_base_dn: d.group_base_dn,
            group_filter: d.group_filter,
            admin_group_dn: d.admin_group_dn,
            enabled: d.enabled,
            managed_by: d.managed_by.into(),
            created_at: d.created_at.timestamp_millis(),
            updated_at: d.updated_at.timestamp_millis(),
        }
    }
}

/// A directory to create, or an unsaved form to test.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewLdapDirectoryBody {
    #[garde(length(chars, min = 1, max = MAX_NAME))]
    pub name: String,
    #[garde(length(max = MAX_URL))]
    pub url: String,
    #[serde(default)]
    #[garde(skip)]
    pub start_tls: bool,
    #[serde(default = "verify_by_default")]
    #[garde(skip)]
    pub tls_verify: bool,
    #[garde(length(chars, max = MAX_DN))]
    pub bind_dn: Option<String>,
    /// Write-only; required with `bind_dn`.
    #[garde(length(max = MAX_BIND_PASSWORD))]
    #[schema(format = Password)]
    pub bind_password: Option<String>,
    #[garde(length(chars, max = MAX_DN))]
    pub user_base_dn: String,
    /// Blank: `(objectClass=person)`.
    #[serde(default)]
    #[garde(length(chars, max = MAX_FILTER))]
    pub user_filter: String,
    /// Blank: `uid`.
    #[serde(default)]
    #[garde(length(chars, max = MAX_ATTR))]
    pub username_attr: String,
    #[garde(length(chars, max = MAX_ATTR))]
    pub email_attr: Option<String>,
    #[garde(length(chars, max = MAX_ATTR))]
    pub display_name_attr: Option<String>,
    #[garde(length(chars, max = MAX_DN))]
    pub group_base_dn: Option<String>,
    #[garde(length(chars, max = MAX_FILTER))]
    pub group_filter: Option<String>,
    #[garde(length(chars, max = MAX_DN))]
    pub admin_group_dn: Option<String>,
    #[serde(default = "enabled_by_default")]
    #[garde(skip)]
    pub enabled: bool,
}

fn verify_by_default() -> bool {
    true
}

fn enabled_by_default() -> bool {
    true
}

impl From<NewLdapDirectoryBody> for NewDirectory {
    fn from(b: NewLdapDirectoryBody) -> Self {
        Self {
            name: b.name,
            url: b.url,
            start_tls: b.start_tls,
            tls_verify: b.tls_verify,
            bind_dn: b.bind_dn,
            bind_password: b.bind_password,
            user_base_dn: b.user_base_dn,
            user_filter: b.user_filter,
            username_attr: b.username_attr,
            email_attr: b.email_attr,
            display_name_attr: b.display_name_attr,
            group_base_dn: b.group_base_dn,
            group_filter: b.group_filter,
            admin_group_dn: b.admin_group_dn,
            enabled: b.enabled,
        }
    }
}

/// Changes to a directory; absent fields stay as they are, `null` clears a nullable one.
/// Changing `url`, `start_tls`, `tls_verify` or `bind_dn` of a directory with a service
/// account needs `bind_password` again.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LdapDirectoryPatch {
    #[garde(length(chars, min = 1, max = MAX_NAME))]
    pub name: Option<String>,
    #[garde(length(max = MAX_URL))]
    pub url: Option<String>,
    #[garde(skip)]
    pub start_tls: Option<bool>,
    #[garde(skip)]
    pub tls_verify: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_DN))]
    pub bind_dn: Option<Option<String>>,
    /// A new password; `null` removes it (with `bind_dn: null`).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, format = Password)]
    #[garde(length(max = MAX_BIND_PASSWORD))]
    pub bind_password: Option<Option<String>>,
    #[garde(length(chars, max = MAX_DN))]
    pub user_base_dn: Option<String>,
    #[garde(length(chars, max = MAX_FILTER))]
    pub user_filter: Option<String>,
    #[garde(length(chars, max = MAX_ATTR))]
    pub username_attr: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_ATTR))]
    pub email_attr: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_ATTR))]
    pub display_name_attr: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_DN))]
    pub group_base_dn: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_FILTER))]
    pub group_filter: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_DN))]
    pub admin_group_dn: Option<Option<String>>,
    #[garde(skip)]
    pub enabled: Option<bool>,
}

impl From<LdapDirectoryPatch> for DirectoryUpdate {
    fn from(p: LdapDirectoryPatch) -> Self {
        Self {
            name: p.name,
            url: p.url,
            start_tls: p.start_tls,
            tls_verify: p.tls_verify,
            bind_dn: p.bind_dn,
            bind_password: p.bind_password,
            user_base_dn: p.user_base_dn,
            user_filter: p.user_filter,
            username_attr: p.username_attr,
            email_attr: p.email_attr,
            display_name_attr: p.display_name_attr,
            group_base_dn: p.group_base_dn,
            group_filter: p.group_filter,
            admin_group_dn: p.admin_group_dn,
            enabled: p.enabled,
        }
    }
}
