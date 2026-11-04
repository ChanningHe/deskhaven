//! The client options the strategy editor knows (`strategy_catalog`, M3-07): every option
//! the official client keeps in `Config::options`, which is where a strategy's
//! `config_options` land, with its control type, client default, group and en / zh texts.
//!
//! The data ([`options`]) is generated from the client source by `cargo xtask
//! gen-strategy-catalog` (keys and the defaults `option2bool` implies) merged with the
//! human-written `overlay.toml` next to it. Options outside the catalog are still valid in
//! a strategy (the advanced editor); the catalog only types the ones it knows.
//!
//! **Secret options** ([`CatalogOption::secret`]): their value reaches every device of the
//! strategy in clear (see the parent module), but the console never shows it again and
//! audit rows mask it ([`is_secret`]).

// Formatted by the generator: `cargo fmt` must not touch it, or `--check` would fail.
#[rustfmt::skip]
mod generated;

/// English and Chinese text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub en: &'static str,
    pub zh: &'static str,
}

/// One allowed value of a [`OptionKind::Choice`] option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub value: &'static str,
    pub label: Text,
}

/// How the console edits an option. An empty value is always allowed besides: it makes the
/// client delete the option (back to its own default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// `Y` or `N`.
    Bool,
    /// One of the listed values.
    Choice(&'static [Choice]),
    /// A decimal integer within the bounds.
    Number { min: Option<i64>, max: Option<i64> },
    /// Free text.
    Text,
}

/// Editor section of an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    Security,
    Permissions,
    Display,
    Network,
    Recording,
    Presets,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogOption {
    pub key: &'static str,
    pub kind: OptionKind,
    pub group: Group,
    /// What the client does when the option is unset; `None` when the source does not say.
    pub default: Option<&'static str>,
    /// The value is a credential: never shown back, masked in audit rows.
    pub secret: bool,
    /// Left out of the typed form of the console (custom-client settings); still valid in a
    /// strategy through the advanced editor.
    pub advanced_only: bool,
    pub label: Text,
    pub description: Text,
}

/// Client revisions the catalog was generated from (`rustdesk/rustdesk`,
/// `rustdesk/hbb_common` commits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Revisions {
    pub rustdesk: &'static str,
    pub hbb_common: &'static str,
}

/// Every known option, in the client's key list order.
pub fn options() -> &'static [CatalogOption] {
    generated::OPTIONS
}

pub fn source() -> Revisions {
    generated::SOURCE
}

pub fn find(key: &str) -> Option<&'static CatalogOption> {
    generated::OPTIONS.iter().find(|o| o.key == key)
}

/// Whether the value of option `key` is a credential.
pub fn is_secret(key: &str) -> bool {
    find(key).is_some_and(|o| o.secret)
}

/// Why `value` does not fit option `key`; `None` when it does or the option is unknown.
/// The empty value (delete) always fits. Never echoes the value.
pub fn value_problem(key: &str, value: &str) -> Option<&'static str> {
    let option = find(key)?;
    if value.is_empty() {
        return None;
    }
    match option.kind {
        OptionKind::Bool if value != "Y" && value != "N" => Some("must be Y or N"),
        OptionKind::Choice(choices) if !choices.iter().any(|c| c.value == value) => {
            Some("is not one of the allowed values")
        }
        OptionKind::Number { min, max } => match value.parse::<i64>() {
            Err(_) => Some("must be a whole number"),
            Ok(n) if min.is_some_and(|lo| n < lo) || max.is_some_and(|hi| n > hi) => {
                Some("is out of range")
            }
            Ok(_) => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn keys_are_unique_and_policy_options_are_present() {
        let keys: HashSet<_> = options().iter().map(|o| o.key).collect();
        assert_eq!(keys.len(), options().len());
        for key in [
            "approve-mode",
            "verification-method",
            "allow-remote-config-modification",
            "enable-keyboard",
            "enable-file-transfer",
            "whitelist",
            "direct-server",
            "direct-access-port",
        ] {
            assert!(keys.contains(key), "{key}");
        }
        assert_eq!(source().rustdesk.len(), 7);
    }

    #[test]
    fn credentials_are_secret_and_nothing_else_is() {
        let secret: Vec<_> = options()
            .iter()
            .filter(|o| o.secret)
            .map(|o| o.key)
            .collect();
        assert_eq!(
            secret,
            [
                "proxy-password",
                "preset-address-book-password",
                "ice-servers"
            ]
        );
        let advanced: Vec<_> = options()
            .iter()
            .filter(|o| o.advanced_only)
            .map(|o| o.key)
            .collect();
        assert_eq!(advanced, ["proxy-url", "proxy-username", "proxy-password"]);
        assert!(is_secret("proxy-password"));
        assert!(!is_secret("temporary-password-length"));
        assert!(!is_secret("not-in-the-catalog"));
    }

    #[test]
    fn defaults_fit_their_kind() {
        for option in options() {
            if let Some(default) = option.default {
                assert_eq!(value_problem(option.key, default), None, "{}", option.key);
            }
        }
        assert_eq!(find("enable-keyboard").and_then(|o| o.default), Some("Y"));
        assert_eq!(find("allow-websocket").and_then(|o| o.default), Some("N"));
    }

    #[test]
    fn values_are_checked_against_the_kind() {
        let cases = [
            ("enable-keyboard", "Y", None),
            ("enable-keyboard", "", None),
            ("enable-keyboard", "yes", Some("must be Y or N")),
            ("approve-mode", "click", None),
            (
                "approve-mode",
                "tap",
                Some("is not one of the allowed values"),
            ),
            ("direct-access-port", "21118", None),
            ("direct-access-port", "0", Some("is out of range")),
            ("direct-access-port", "65536", Some("is out of range")),
            ("direct-access-port", "21x", Some("must be a whole number")),
            ("whitelist", "10.0.0.0/8, 192.0.2.1", None),
            ("custom-key", "anything", None),
        ];
        for (key, value, want) in cases {
            assert_eq!(value_problem(key, value), want, "{key}={value}");
        }
    }
}
