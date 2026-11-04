//! Identity configuration (M3-09): OIDC providers, the LDAP directory and its diagnostics.
//! (The 2FA enforcement policy is the `auth.tfa.enforce` setting, edited through
//! `/settings` like every setting, pins and versions included.)
//!
//! | route | permission |
//! |---|---|
//! | `GET /oidc-providers`, `GET /oidc-providers/{id}` | admin |
//! | `POST /oidc-providers`, `PATCH`/`DELETE /oidc-providers/{id}` | admin |
//! | `GET /ldap-directories`, `GET /ldap-directories/{id}` | admin |
//! | `POST /ldap-directories`, `PATCH`/`DELETE /ldap-directories/{id}` | owner |
//! | `POST /ldap-tests/connection`, `POST /ldap-tests/user` | owner |
//!
//! Secrets (client secret, bind password) are write-only: never in a response or an audit
//! row. Resources from the configuration file (`managed_by = config`) are read-only (409
//! `managed_by_config`). `admin_group` makes provisioned accounts admins: it is only safe
//! when the provider's group claim is controlled solely by the IdP's administrators, never
//! with a multi-tenant IdP or one where users edit their own groups.

mod diagnostics;
mod diagnostics_view;
mod error;
mod guard;
mod ldap;
mod ldap_view;
mod oidc;
mod oidc_view;

use utoipa_axum::routes;

use crate::{AdminRoutes, RouteError};

/// The `/oidc-providers`, `/ldap-directories` and `/ldap-tests` routes. Reads and writes
/// of one directory path are separate calls: they need different permissions.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(
        oidc::list_oidc_provider_configs,
        oidc::create_oidc_provider
    ))?
    .routes(routes!(
        oidc::get_oidc_provider,
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
