//! `admin_group` makes provisioned accounts admins: it is only safe when the IdP's
//! administrators alone control the group claim, never with a multi-tenant IdP.

mod diagnostics;
mod diagnostics_view;
mod error;
mod guard;
#[cfg(test)]
mod guard_tests;
mod ldap;
mod ldap_view;
mod oidc;
mod oidc_view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// Reads and writes of one path are separate calls: they need different permissions.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(oidc::list_oidc_provider_configs))?
        .routes(routes!(oidc::create_oidc_provider))?
        .routes(routes!(oidc::get_oidc_provider))?
        .routes(routes!(
            oidc::update_oidc_provider,
            oidc::delete_oidc_provider
        ))?
        .routes(routes!(ldap::list_ldap_directories))?
        .routes(routes!(ldap::create_ldap_directory))?
        .routes(routes!(ldap::get_ldap_directory))?
        .routes(routes!(
            ldap::update_ldap_directory,
            ldap::delete_ldap_directory
        ))?
        .routes(routes!(diagnostics::test_ldap_connection))?
        .routes(routes!(diagnostics::test_ldap_user))
}
