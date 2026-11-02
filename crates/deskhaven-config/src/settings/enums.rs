//! Fixed-variant setting values. Each enum is declared once with its wire
//! names; parsing, display and the schema's `variants` derive from that.

use serde_json::{Map, Value as Json};

use super::kind::{SettingType, sealed};
use super::value::Value;

macro_rules! setting_enum {
    (
        $(#[doc = $doc:literal])+
        $name:ident { $( $variant:ident = $text:literal ),+ $(,)? }
    ) => {
        $(#[doc = $doc])+
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $( #[doc = concat!("`", $text, "`")] $variant ),+
        }

        impl $name {
            /// Wire names, in declaration order.
            pub const VARIANTS: &'static [&'static str] = &[$($text),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $text ),+
                }
            }

            fn from_name(name: &str) -> Option<Self> {
                match name {
                    $( $text => Some(Self::$variant), )+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl sealed::Sealed for $name {}

        impl SettingType for $name {
            type Default = Self;
            type Rule = ();
            const DEFAULT_RULE: () = ();
            const TYPE_NAME: &'static str = "enum";

            fn parse(raw: &Json) -> Result<Self, String> {
                raw.as_str()
                    .and_then(|s| Self::from_name(s.trim()))
                    .ok_or_else(|| format!("expected one of: {}", Self::VARIANTS.join(", ")))
            }
            fn check(&self, _: &()) -> Result<(), String> {
                Ok(())
            }
            fn from_default(default: &Self) -> Self {
                *default
            }
            fn into_value(self) -> Value {
                Value::Enum(self.as_str())
            }
            fn from_value(value: &Value) -> Option<Self> {
                match value {
                    Value::Enum(name) => Self::from_name(name),
                    _ => None,
                }
            }
            fn describe(_: &()) -> Map<String, Json> {
                let variants = Self::VARIANTS.iter().map(|v| Json::from(*v)).collect();
                Map::from_iter([("variants".to_owned(), Json::Array(variants))])
            }
        }
    };
}

setting_enum! {
    /// Who may create an account without an administrator.
    Registration { Disabled = "disabled", Invite = "invite", Open = "open" }
}

setting_enum! {
    /// Which users must enroll a second factor.
    TfaEnforce { Off = "off", Admins = "admins", All = "all" }
}

setting_enum! {
    /// SMTP transport security.
    SmtpTls { Starttls = "starttls", Tls = "tls", None = "none" }
}

setting_enum! {
    /// Outcome of a connection ACL decision.
    AclAction { Allow = "allow", Deny = "deny" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_wire_names_only() {
        assert_eq!(
            Registration::parse(&json!("invite")),
            Ok(Registration::Invite)
        );
        assert_eq!(
            Registration::parse(&json!(" open ")),
            Ok(Registration::Open)
        );
        assert_eq!(
            Registration::parse(&json!("Invite")),
            Err("expected one of: disabled, invite, open".to_owned())
        );
        assert!(SmtpTls::parse(&json!(1)).is_err());
    }

    #[test]
    fn round_trips_through_value() {
        for tls in [SmtpTls::Starttls, SmtpTls::Tls, SmtpTls::None] {
            assert_eq!(SmtpTls::from_value(&tls.into_value()), Some(tls));
            assert_eq!(tls.to_string(), tls.as_str());
        }
    }
}
