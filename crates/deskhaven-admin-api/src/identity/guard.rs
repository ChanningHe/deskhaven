//! Audit guards of identity writes, decided in the write transaction on the committed
//! role: providers are an administrator's, directories the owner's alone (06 §3.6: whoever
//! can point the directory elsewhere can impersonate every directory account).
//!
//! Snapshots name every non-secret field, so url, bind DN, TLS and enabled changes show in
//! `before` / `after`. A secret appears only as its key with `null` (none) or a marker the
//! audit log masks (set); its value never reaches a snapshot.

use deskhaven_core::console_audit::{Refused, TxGuard};
use deskhaven_core::ldap::Directory;
use deskhaven_core::oidc::Provider;
use deskhaven_core::state::AppState;
use deskhaven_core::users::Role;
use serde::Serialize;

use crate::domain_error::FORBIDDEN;
use crate::{Audit, AuditCtx};

/// Stands for a stored secret in a snapshot (masked by the audit log's key rules anyway).
const SET: &str = "set";

pub(super) fn provider_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
) -> TxGuard<Provider> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&Provider>, after: Option<&Provider>| {
            let id = after.or(before).map_or(0, |p| p.id);
            let mut audit = Audit::new(action).target("oidc_provider", id);
            if let Some(before) = before {
                audit = audit.before(&ProviderSnapshot::from(before));
            }
            if let Some(after) = after {
                audit = audit.after(&ProviderSnapshot::from(after));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

pub(super) fn directory_guard(
    state: &AppState,
    ctx: &AuditCtx,
    action: &'static str,
) -> TxGuard<Directory> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |role, before: Option<&Directory>, after: Option<&Directory>| {
            if role != Role::Owner {
                return Err(Refused::new(FORBIDDEN));
            }
            let id = after.or(before).map_or(0, |d| d.id);
            let mut audit = Audit::new(action).target("ldap_directory", id);
            if let Some(before) = before {
                audit = audit.before(&DirectorySnapshot::from(before));
            }
            if let Some(after) = after {
                audit = audit.after(&DirectorySnapshot::from(after));
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

fn secret_marker(set: bool) -> Option<&'static str> {
    set.then_some(SET)
}

#[derive(Serialize)]
struct ProviderSnapshot<'a> {
    name: &'a str,
    display_name: &'a str,
    issuer: &'a str,
    client_id: &'a str,
    client_secret: Option<&'static str>,
    scopes: &'a str,
    auto_provision: bool,
    group_claim: Option<&'a str>,
    admin_group: Option<&'a str>,
    enabled: bool,
}

impl<'a> From<&'a Provider> for ProviderSnapshot<'a> {
    fn from(p: &'a Provider) -> Self {
        Self {
            name: &p.name,
            display_name: &p.display_name,
            issuer: &p.issuer,
            client_id: &p.client_id,
            client_secret: secret_marker(p.has_client_secret),
            scopes: &p.scopes,
            auto_provision: p.auto_provision,
            group_claim: p.group_claim.as_deref(),
            admin_group: p.admin_group.as_deref(),
            enabled: p.enabled,
        }
    }
}

#[derive(Serialize)]
struct DirectorySnapshot<'a> {
    name: &'a str,
    url: &'a str,
    start_tls: bool,
    tls_verify: bool,
    bind_dn: Option<&'a str>,
    bind_password: Option<&'static str>,
    user_base_dn: &'a str,
    user_filter: &'a str,
    username_attr: &'a str,
    group_base_dn: Option<&'a str>,
    group_filter: Option<&'a str>,
    admin_group_dn: Option<&'a str>,
    enabled: bool,
}

impl<'a> From<&'a Directory> for DirectorySnapshot<'a> {
    fn from(d: &'a Directory) -> Self {
        Self {
            name: &d.name,
            url: &d.url,
            start_tls: d.start_tls,
            tls_verify: d.tls_verify,
            bind_dn: d.bind_dn.as_deref(),
            bind_password: secret_marker(d.has_bind_password),
            user_base_dn: &d.user_base_dn,
            user_filter: &d.user_filter,
            username_attr: &d.username_attr,
            group_base_dn: d.group_base_dn.as_deref(),
            group_filter: d.group_filter.as_deref(),
            admin_group_dn: d.admin_group_dn.as_deref(),
            enabled: d.enabled,
        }
    }
}
