//! Value kinds: how each Rust type is parsed, constrained and described.
//!
//! Parsing accepts the native JSON form or its text form, so TOML pins
//! (native), env/`_FILE`/CLI pins (always text), stored rows and API input
//! all go through one coercion. Error reasons never contain the input.

use std::fmt;

use serde_json::{Map, Value as Json};

use super::text::{TextRule, max_len};
use super::value::{HostPort, Value};
use crate::{Duration, Secret};

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A Rust type usable as a setting value. Sealed: the set of kinds is fixed
/// here (enums join through `setting_enum!`).
pub trait SettingType: sealed::Sealed + Clone + Send + Sync + Sized + 'static {
    /// Const-constructible form of the default.
    type Default: Send + Sync + 'static;
    /// Constraints checked after parsing.
    type Rule: Send + Sync + 'static;
    /// Rule used when a declaration gives none.
    const DEFAULT_RULE: Self::Rule;
    /// The `type` field of the exported schema.
    const TYPE_NAME: &'static str;
    /// Stored encrypted and never shown.
    const SENSITIVE: bool = false;

    fn parse(raw: &Json) -> Result<Self, String>;
    fn check(&self, rule: &Self::Rule) -> Result<(), String>;
    fn from_default(default: &Self::Default) -> Self;
    fn into_value(self) -> Value;
    fn from_value(value: &Value) -> Option<Self>;
    /// Schema fields describing the rule (and variants, for enums).
    fn describe(rule: &Self::Rule) -> Map<String, Json>;
}

/// Inclusive bounds; with [`Range::or_zero`] zero is accepted too ("off" /
/// "forever").
#[derive(Debug, Clone, Copy)]
pub struct Range<T> {
    min: T,
    max: T,
    zero: bool,
}

/// `min..=max`.
pub const fn range<T>(min: T, max: T) -> Range<T> {
    Range {
        min,
        max,
        zero: false,
    }
}

impl<T: Copy> Range<T> {
    pub const fn or_zero(self) -> Self {
        Self {
            min: self.min,
            max: self.max,
            zero: true,
        }
    }
}

/// Numeric-like types that can be bounded by a [`Range`].
pub trait Bound: Copy + PartialOrd + fmt::Display {
    fn is_zero(&self) -> bool;
    fn to_json(&self) -> Json;
}

impl Bound for u32 {
    fn is_zero(&self) -> bool {
        *self == 0
    }
    fn to_json(&self) -> Json {
        Json::from(*self)
    }
}

impl Bound for Duration {
    fn is_zero(&self) -> bool {
        self.as_std().is_zero()
    }
    fn to_json(&self) -> Json {
        Json::String(self.to_string())
    }
}

impl<T: Bound> Range<T> {
    fn check(&self, value: T) -> Result<(), String> {
        if (self.zero && value.is_zero()) || (self.min <= value && value <= self.max) {
            return Ok(());
        }
        let zero = if self.zero { " (or 0)" } else { "" };
        Err(format!(
            "must be between {} and {}{zero}",
            self.min, self.max
        ))
    }

    fn describe(&self) -> Map<String, Json> {
        let mut map = Map::new();
        map.insert("min".into(), self.min.to_json());
        map.insert("max".into(), self.max.to_json());
        map.insert("zero_allowed".into(), Json::Bool(self.zero));
        map
    }
}

/// List constraints.
#[derive(Debug, Clone, Copy)]
pub struct ListRule {
    max_items: usize,
}

pub const fn max_items(max_items: usize) -> ListRule {
    ListRule { max_items }
}

const DEFAULT_TEXT_RULE: TextRule = max_len(255);
const DEFAULT_LIST_RULE: ListRule = max_items(32);
const EXPECTED_STRING: &str = "expected a string";

/// ASCII digits only: no sign, no blanks (`u32::from_str` accepts `+5`).
fn parse_digits(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn as_text(raw: &Json) -> Result<&str, String> {
    raw.as_str().ok_or_else(|| EXPECTED_STRING.to_owned())
}

impl sealed::Sealed for bool {}
impl SettingType for bool {
    type Default = bool;
    type Rule = ();
    const DEFAULT_RULE: () = ();
    const TYPE_NAME: &'static str = "bool";

    fn parse(raw: &Json) -> Result<Self, String> {
        match raw {
            Json::Bool(b) => Ok(*b),
            Json::String(s) if s.trim().eq_ignore_ascii_case("true") => Ok(true),
            Json::String(s) if s.trim().eq_ignore_ascii_case("false") => Ok(false),
            _ => Err("expected true or false".to_owned()),
        }
    }
    fn check(&self, _: &()) -> Result<(), String> {
        Ok(())
    }
    fn from_default(default: &bool) -> Self {
        *default
    }
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    fn describe(_: &()) -> Map<String, Json> {
        Map::new()
    }
}

impl sealed::Sealed for u32 {}
impl SettingType for u32 {
    type Default = u32;
    type Rule = Range<u32>;
    const DEFAULT_RULE: Range<u32> = range(0, u32::MAX);
    const TYPE_NAME: &'static str = "u32";

    fn parse(raw: &Json) -> Result<Self, String> {
        let parsed = match raw {
            Json::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            Json::String(s) => parse_digits(s.trim()),
            _ => None,
        };
        parsed.ok_or_else(|| "expected a whole number".to_owned())
    }
    fn check(&self, rule: &Range<u32>) -> Result<(), String> {
        rule.check(*self)
    }
    fn from_default(default: &u32) -> Self {
        *default
    }
    fn into_value(self) -> Value {
        Value::U32(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::U32(n) => Some(*n),
            _ => None,
        }
    }
    fn describe(rule: &Range<u32>) -> Map<String, Json> {
        rule.describe()
    }
}

impl sealed::Sealed for Duration {}
impl SettingType for Duration {
    type Default = Duration;
    type Rule = Range<Duration>;
    const DEFAULT_RULE: Range<Duration> = range(
        Duration::from_std(std::time::Duration::ZERO),
        Duration::from_std(std::time::Duration::MAX),
    );
    const TYPE_NAME: &'static str = "duration";

    /// Strings only: a bare number has no unit (05-config §2.3).
    fn parse(raw: &Json) -> Result<Self, String> {
        as_text(raw)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| "expected a duration with units, e.g. \"15m\" or \"7d\"".to_owned())
    }
    fn check(&self, rule: &Range<Duration>) -> Result<(), String> {
        rule.check(*self)
    }
    fn from_default(default: &Duration) -> Self {
        *default
    }
    fn into_value(self) -> Value {
        Value::Duration(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Duration(d) => Some(*d),
            _ => None,
        }
    }
    fn describe(rule: &Range<Duration>) -> Map<String, Json> {
        rule.describe()
    }
}

impl sealed::Sealed for String {}
impl SettingType for String {
    type Default = &'static str;
    type Rule = TextRule;
    const DEFAULT_RULE: TextRule = DEFAULT_TEXT_RULE;
    const TYPE_NAME: &'static str = "string";

    fn parse(raw: &Json) -> Result<Self, String> {
        as_text(raw).map(|s| s.trim().to_owned())
    }
    fn check(&self, rule: &TextRule) -> Result<(), String> {
        rule.check(self)
    }
    fn from_default(default: &&'static str) -> Self {
        (*default).to_owned()
    }
    fn into_value(self) -> Value {
        Value::Text(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Text(text) => Some(text.clone()),
            _ => None,
        }
    }
    fn describe(rule: &TextRule) -> Map<String, Json> {
        rule.describe()
    }
}

/// Defaults to unset; secrets are taken verbatim (no trimming).
impl sealed::Sealed for Secret {}
impl SettingType for Secret {
    type Default = ();
    type Rule = TextRule;
    const DEFAULT_RULE: TextRule = DEFAULT_TEXT_RULE;
    const TYPE_NAME: &'static str = "secret";
    const SENSITIVE: bool = true;

    fn parse(raw: &Json) -> Result<Self, String> {
        as_text(raw).map(Secret::new)
    }
    fn check(&self, rule: &TextRule) -> Result<(), String> {
        rule.check(self.expose())
    }
    fn from_default(_: &()) -> Self {
        Secret::default()
    }
    fn into_value(self) -> Value {
        Value::Secret(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        value.secret().cloned()
    }
    fn describe(rule: &TextRule) -> Map<String, Json> {
        rule.describe()
    }
}

/// `["a:1", "b:2"]` or the text form `"a:1,b:2"`; defaults to empty.
impl sealed::Sealed for Vec<HostPort> {}
impl SettingType for Vec<HostPort> {
    type Default = ();
    type Rule = ListRule;
    const DEFAULT_RULE: ListRule = DEFAULT_LIST_RULE;
    const TYPE_NAME: &'static str = "host_port_list";

    fn parse(raw: &Json) -> Result<Self, String> {
        let items: Vec<&str> = match raw {
            Json::Array(items) => items.iter().map(as_text).collect::<Result<_, _>>()?,
            // Blank text is the empty list (an env var can clear the list);
            // otherwise every comma-separated item counts, empty ones too.
            Json::String(s) if s.trim().is_empty() => Vec::new(),
            Json::String(s) => s.split(',').map(str::trim).collect(),
            _ => return Err("expected a list of host:port".to_owned()),
        };
        items
            .iter()
            .enumerate()
            .map(|(i, item)| item.parse().map_err(|e| format!("item {}: {e}", i + 1)))
            .collect()
    }
    fn check(&self, rule: &ListRule) -> Result<(), String> {
        if self.len() > rule.max_items {
            return Err(format!("must have at most {} items", rule.max_items));
        }
        match self
            .iter()
            .enumerate()
            .find(|(i, hp)| self[..*i].contains(hp))
        {
            Some((i, _)) => Err(format!("item {} is a duplicate", i + 1)),
            None => Ok(()),
        }
    }
    fn from_default(_: &()) -> Self {
        Vec::new()
    }
    fn into_value(self) -> Value {
        Value::HostPorts(self)
    }
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::HostPorts(list) => Some(list.clone()),
            _ => None,
        }
    }
    fn describe(rule: &ListRule) -> Map<String, Json> {
        Map::from_iter([
            ("item".to_owned(), Json::from("host:port")),
            ("max_items".to_owned(), Json::from(rule.max_items)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bool_accepts_native_and_text() {
        assert_eq!(bool::parse(&json!(true)), Ok(true));
        assert_eq!(bool::parse(&json!(" FALSE ")), Ok(false));
        assert!(bool::parse(&json!("yes")).is_err());
        assert!(bool::parse(&json!(1)).is_err());
    }

    #[test]
    fn u32_accepts_native_and_text_within_range() {
        assert_eq!(u32::parse(&json!(10)), Ok(10));
        assert_eq!(u32::parse(&json!("010")), Ok(10));
        assert!(u32::parse(&json!(-1)).is_err());
        assert!(u32::parse(&json!(4_294_967_296_u64)).is_err());
        assert!(u32::parse(&json!(1.5)).is_err());
        let rule = range(1, 5);
        assert!(3.check(&rule).is_ok());
        assert_eq!(9.check(&rule), Err("must be between 1 and 5".to_owned()));
        assert!(0.check(&rule.or_zero()).is_ok());
    }

    #[test]
    fn duration_rejects_bare_numbers() {
        assert!(Duration::parse(&json!(60)).is_err());
        assert!(Duration::parse(&json!("60")).is_err());
        assert!(Duration::parse(&json!("1h30m")).is_ok());
    }

    #[test]
    fn u32_text_is_digits_only() {
        assert!(u32::parse(&json!("+5")).is_err());
        assert!(u32::parse(&json!("")).is_err());
        assert_eq!(u32::parse(&json!(" 7 ")), Ok(7));
    }

    #[test]
    fn text_is_trimmed_and_checked() {
        assert_eq!(String::parse(&json!(" a ")), Ok("a".to_owned()));
        assert!("a\nb".to_owned().check(&max_len(9)).is_err());
        assert!(String::parse(&json!(5)).is_err());
    }

    #[test]
    fn host_port_list_accepts_array_or_comma_text() {
        assert!(Vec::<HostPort>::parse(&json!("a:1,")).is_err());
        let from_text = Vec::<HostPort>::parse(&json!(" a:1 , b:2 ")).unwrap();
        let from_array = Vec::<HostPort>::parse(&json!(["a:1", "b:2"])).unwrap();
        assert_eq!(from_text, from_array);
        assert_eq!(Vec::<HostPort>::parse(&json!("")).unwrap(), vec![]);
        assert_eq!(
            Vec::<HostPort>::parse(&json!(["a:1", "nope"])),
            Err("item 2: expected host:port (DNS name, IPv4 or [IPv6], port 1-65535)".to_owned())
        );
        let dup = Vec::<HostPort>::parse(&json!("a:1,a:1")).unwrap();
        assert_eq!(
            dup.check(&max_items(5)),
            Err("item 2 is a duplicate".to_owned())
        );
        assert!(dup.check(&max_items(1)).is_err());
    }
}
