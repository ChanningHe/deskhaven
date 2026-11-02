//! Strongly typed configuration values with human-friendly string forms.

mod cidr;
mod duration;
mod secret;
mod size;

pub use cidr::Cidr;
pub use duration::Duration;
pub use secret::{REDACTED, Secret};
pub use size::Size;

/// Implements `Serialize`/`Deserialize` via `Display`/`FromStr`, accepting
/// strings only (bare numbers are rejected to avoid unit ambiguity).
macro_rules! string_serde {
    ($ty:ty, $expecting:literal) => {
        impl serde::Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl serde::de::Visitor<'_> for V {
                    type Value = $ty;
                    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$ty, E> {
                        v.parse().map_err(E::custom)
                    }
                }
                d.deserialize_str(V)
            }
        }
    };
}

/// Implements `JsonSchema` as a plain string with a description and example.
macro_rules! string_schema {
    ($ty:ty, $name:literal, $description:literal, $example:literal) => {
        impl schemars::JsonSchema for $ty {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "description": $description,
                    "examples": [$example],
                })
            }
        }
    };
}

pub(crate) use string_schema;
pub(crate) use string_serde;
