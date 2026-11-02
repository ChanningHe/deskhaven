use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Placeholder emitted wherever a secret would otherwise be shown.
pub const REDACTED: &str = "<redacted>";

/// A sensitive string. `Debug` and `Serialize` never reveal the value; an
/// empty secret renders as `""` so "unset" stays distinguishable.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the plaintext. Call only where the value is actually used.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn rendered(&self) -> &'static str {
        if self.0.is_empty() { "" } else { REDACTED }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({:?})", self.rendered())
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.rendered())
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self)
    }
}

impl schemars::JsonSchema for Secret {
    fn schema_name() -> Cow<'static, str> {
        "Secret".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "writeOnly": true,
            "description": "Sensitive value; may also be supplied via the <KEY>_FILE env variant",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_serialize_redact() {
        let s = Secret::new("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(\"<redacted>\")");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"<redacted>\"");
        assert_eq!(s.expose(), "hunter2");
    }

    #[test]
    fn empty_secret_renders_empty() {
        assert_eq!(serde_json::to_string(&Secret::default()).unwrap(), "\"\"");
    }
}
