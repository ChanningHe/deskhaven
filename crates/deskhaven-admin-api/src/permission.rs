//! The declaration is the single source of truth: route assembly reads it back from the
//! generated operation (none fails assembly, so the server does not start), the gate
//! enforces it, and the OpenAPI document shows it.

use deskhaven_core::users::Role;
use serde_json::Value;
use utoipa::openapi::path::Operation;

pub const SESSION_SCHEME: &str = "console_session";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Public,
    /// Also a session that must finish its account setup first.
    Session,
    SelfService,
    Admin,
    Owner,
}

impl Permission {
    const SCOPED: [Self; 4] = [Self::Session, Self::SelfService, Self::Admin, Self::Owner];

    pub const fn scope(self) -> Option<&'static str> {
        match self {
            Self::Public => None,
            Self::Session => Some("session"),
            Self::SelfService => Some("self"),
            Self::Admin => Some("admin"),
            Self::Owner => Some("owner"),
        }
    }

    pub fn allows(self, role: Role) -> bool {
        match self {
            Self::Public | Self::Session | Self::SelfService => true,
            Self::Admin => matches!(role, Role::Admin | Role::Owner),
            Self::Owner => role == Role::Owner,
        }
    }

    pub const fn admits_setup(self) -> bool {
        matches!(self, Self::Public | Self::Session)
    }

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
            _ => Err("declare exactly one role: session, self, admin or owner"),
        }
    }

    fn from_scope(scope: &str) -> Result<Self, &'static str> {
        Self::SCOPED
            .into_iter()
            .find(|p| p.scope() == Some(scope))
            .ok_or("unknown role (expected session, self, admin or owner)")
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
            (scoped(&["session"]), Permission::Session),
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
            (Permission::Session, [true, true, true]),
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
