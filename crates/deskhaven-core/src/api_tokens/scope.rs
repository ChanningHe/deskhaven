//! What a token may do (04 §3 `api_tokens.scopes`, a JSON array of these names).

use std::collections::BTreeSet;

/// One permission of an API token. Scopes are independent: none implies another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// `POST /api/devices/deploy` (`rustdesk --deploy`).
    Deploy,
    /// `POST /api/devices/cli` (`rustdesk --assign`).
    Assign,
    /// Read the admin API.
    AdminRead,
    /// Write through the admin API.
    AdminWrite,
}

impl Scope {
    pub const ALL: [Self; 4] = [
        Self::Deploy,
        Self::Assign,
        Self::AdminRead,
        Self::AdminWrite,
    ];

    /// The stored and displayed name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Assign => "assign",
            Self::AdminRead => "admin:read",
            Self::AdminWrite => "admin:write",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == name)
    }
}

/// The `scopes` column: a JSON array of names, in [`Scope`] order.
pub(super) fn encode(scopes: &BTreeSet<Scope>) -> String {
    let names: Vec<&str> = scopes.iter().map(|s| s.as_str()).collect();
    serde_json::Value::from(names).to_string()
}

/// The scopes of a stored `scopes` column. Unknown names (a newer version's) and malformed
/// JSON grant nothing: a token never gets more than it can be shown to have.
pub(super) fn decode(stored: &str) -> BTreeSet<Scope> {
    let names: Vec<String> = serde_json::from_str(stored).unwrap_or_default();
    names.iter().filter_map(|name| Scope::parse(name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_round_trips_in_a_stable_order() {
        let scopes = BTreeSet::from([Scope::AdminWrite, Scope::Deploy]);

        let stored = encode(&scopes);

        assert_eq!(stored, r#"["deploy","admin:write"]"#);
        assert_eq!(decode(&stored), scopes);
    }

    #[test]
    fn unknown_names_and_garbage_grant_nothing() {
        assert_eq!(
            decode(r#"["assign","root",7]"#),
            BTreeSet::new(),
            "a malformed array is rejected whole"
        );
        assert_eq!(
            decode(r#"["assign","root"]"#),
            BTreeSet::from([Scope::Assign])
        );
        assert_eq!(decode("not json"), BTreeSet::new());
    }
}
