//! A directory account is a member of exactly the console-managed groups its directory groups
//! name: memberships the directory does not back are removed. Owners are never changed,
//! configuration groups never touched, and groups never created.

use std::collections::BTreeSet;

use super::filter::{dn_key, first_rdn_value, is_under};
use super::settings::DirectorySettings;

pub(crate) fn is_admin(settings: &DirectorySettings, group_dns: &[String]) -> Option<bool> {
    let admin = dn_key(settings.admin_group_dn.as_deref()?);
    Some(group_dns.iter().any(|dn| dn_key(dn) == admin))
}

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
