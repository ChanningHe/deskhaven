//! Route permissions (06 §4.1): every admin API operation declares one, in its
//! `#[utoipa::path]` attribute, as an OpenAPI security requirement:
//!
//! | declaration | permission | who passes |
//! |---|---|---|
//! | `security(())` | [`Permission::Public`] | anyone, no session |
//! | `security(("console_session" = ["self"]))` | [`Permission::SelfService`] | any console user |
//! | `security(("console_session" = ["admin"]))` | [`Permission::Admin`] | admin, owner |
//! | `security(("console_session" = ["owner"]))` | [`Permission::Owner`] | owner |
//!
//! The declaration is the single source of truth: route assembly reads it back from the
//! generated operation (an operation without one, or with anything else, fails assembly,
//! so the server does not start), the gate enforces it, and the published OpenAPI document
//! shows it (OpenAPI 3.1 allows role names as the scopes of non-OAuth schemes).

use deskhaven_core::users::Role;
use serde_json::Value;
use utoipa::openapi::path::Operation;

/// Name of the cookie security scheme in the OpenAPI document.
pub const SESSION_SCHEME: &str = "console_session";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// No session needed (console login, first-run setup). Not audited automatically.
    Public,
    /// Any authenticated console user, acting on their own account (08: "self").
    SelfService,
    /// Administrators and the owner.
    Admin,
    /// The owner only.
    Owner,
}

impl Permission {
    const SCOPED: [Self; 3] = [Self::SelfService, Self::Admin, Self::Owner];

    /// The role name in the security requirement; `None` for [`Permission::Public`].
    pub const fn scope(self) -> Option<&'static str> {
        match self {
            Self::Public => None,
            Self::SelfService => Some("self"),
            Self::Admin => Some("admin"),
            Self::Owner => Some("owner"),
        }
    }

    /// Whether a console user of `role` may call an operation with this permission.
    pub fn allows(self, role: Role) -> bool {
        match self {
            Self::Public | Self::SelfService => true,
            Self::Admin => matches!(role, Role::Admin | Role::Owner),
            Self::Owner => role == Role::Owner,
        }
    }

    /// The permission an operation declares (see the module docs), or why it declares none.
    pub fn of_operation(operation: &Operation) -> Result<Self, &'static str> {
        let declared = operation
            .security
            .as_ref()
            .ok_or("no permission declared (add `security(...)` to #[utoipa::path])")?;
        let [requirement] = declared.as_slice() else {
            return Err("declare exactly one security requirement");
        };
        // The requirement's map is private; its serialized form is `{scheme: [scopes]}`.
        let value = serde_json::to_value(requirement).map_err(|_| "unreadable security")?;
        let Value::Object(schemes) = value else {
            return Err("unreadable security requirement");
        };
        if schemes.is_empty() {
            return Ok(Self::Public);
        }
        let scopes = match schemes.get(SESSION_SCHEME) {
            Some(Value::Array(scopes)) if schemes.len() == 1 => scopes,
            _ => return Err("the only security scheme is `console_session`"),
        };
        match scopes.as_slice() {
            [Value::String(scope)] => Self::from_scope(scope),
            _ => Err("declare exactly one role: self, admin or owner"),
        }
    }

    fn from_scope(scope: &str) -> Result<Self, &'static str> {
        Self::SCOPED
            .into_iter()
            .find(|p| p.scope() == Some(scope))
            .ok_or("unknown role (expected self, admin or owner)")
    }
}

#[cfg(test)]
mod tests {
    use utoipa::openapi::security::SecurityRequirement;

    use super::*;

    fn operation(security: Option<Vec<SecurityRequirement>>) -> Operation {
        let mut op = Operation::new();
        op.security = security;
        op
    }

    fn scoped(scopes: &[&str]) -> Option<Vec<SecurityRequirement>> {
        Some(vec![SecurityRequirement::new(
            SESSION_SCHEME,
            scopes.iter().copied(),
        )])
    }

    #[test]
    fn reads_each_declaration() {
        let cases = [
            (
                Some(vec![SecurityRequirement::default()]),
                Permission::Public,
            ),
            (scoped(&["self"]), Permission::SelfService),
            (scoped(&["admin"]), Permission::Admin),
            (scoped(&["owner"]), Permission::Owner),
        ];
        for (security, expected) in cases {
            assert_eq!(Permission::of_operation(&operation(security)), Ok(expected));
        }
    }

    #[test]
    fn rejects_missing_or_malformed_declarations() {
        let other = Some(vec![SecurityRequirement::new("api_key", ["admin"])]);
        let both = Some(vec![
            SecurityRequirement::default(),
            SecurityRequirement::new(SESSION_SCHEME, ["admin"]),
        ]);
        for security in [
            None,
            Some(vec![]),
            scoped(&[]),
            scoped(&["admin", "owner"]),
            scoped(&["superuser"]),
            other,
            both,
        ] {
            let got = Permission::of_operation(&operation(security.clone()));
            assert!(got.is_err(), "{:?}", serde_json::to_value(&security));
        }
    }

    #[test]
    fn role_matrix() {
        use Role::{Admin, Owner, User};
        let expect = [
            (Permission::Public, [true, true, true]),
            (Permission::SelfService, [true, true, true]),
            (Permission::Admin, [false, true, true]),
            (Permission::Owner, [false, false, true]),
        ];
        for (permission, allowed) in expect {
            let got = [User, Admin, Owner].map(|role| permission.allows(role));
            assert_eq!(got, allowed, "{permission:?}");
        }
    }
}
