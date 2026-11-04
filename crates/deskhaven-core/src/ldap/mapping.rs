//! What a user's directory groups mean in deskhaven (06 §3.5), applied at every login:
//!
//! - **admin flag** (with `admin_group_dn`): `admin` when one of the group DNs is the admin
//!   group's (DNs compare by RDN, ASCII case-insensitively), `user` otherwise. Owners are
//!   never changed. Without an admin group the role is the console's.
//! - **groups** (with `group_base_dn`): the directory groups below the base, by the value
//!   of their first RDN (`cn=ops,ou=groups,...` -> `ops`), name deskhaven groups
//!   (case-insensitively). A directory account is a member of exactly the console-managed
//!   (`ui`) groups so named: memberships the directory does not back are removed, also ones
//!   added in the console. Configuration-managed groups are never touched, and groups are
//!   never created. Without a group base the memberships are the console's.

use std::collections::BTreeSet;

use super::filter::{dn_key, first_rdn_value, is_under};
use super::settings::DirectorySettings;

/// `Some(is admin)` when the directory has an admin group.
pub(crate) fn is_admin(settings: &DirectorySettings, group_dns: &[String]) -> Option<bool> {
    let admin = dn_key(settings.admin_group_dn.as_deref()?);
    Some(group_dns.iter().any(|dn| dn_key(dn) == admin))
}

/// The lowercased deskhaven group names the groups map to, when group mapping is on.
pub(crate) fn group_names(
    settings: &DirectorySettings,
    group_dns: &[String],
) -> Option<BTreeSet<String>> {
    let base = settings.group_base_dn.as_deref()?;
    let names = group_dns
        .iter()
        .filter(|dn| is_under(dn, base))
        .filter_map(|dn| first_rdn_value(dn))
        .map(|name| name.to_lowercase())
        .collect();
    Some(names)
}
