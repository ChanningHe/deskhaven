//! One setting declaration: key, description, typed default, rule and
//! bad-row fallback.

use std::marker::PhantomData;

use serde_json::{Map, Value as Json};

use super::kind::SettingType;
use super::value::Value;

/// A declared setting whose values are `T`. Declared only through
/// `settings!` (see `registry.rs`); read with
/// [`Snapshot::get`](super::Snapshot::get).
pub struct Setting<T: SettingType> {
    key: &'static str,
    description: &'static str,
    default: T::Default,
    rule: T::Rule,
    /// Value used when a stored row exists but cannot be used; `None` =
    /// the default. Set for security-relevant settings, so corruption or a
    /// lost master key fails closed rather than open.
    on_bad_row: Option<T::Default>,
    _type: PhantomData<fn() -> T>,
}

/// Bad-row policy: fall back to `value` instead of the default.
pub(crate) const fn strict<D>(value: D) -> Option<D> {
    Some(value)
}

impl<T: SettingType> Setting<T> {
    pub(crate) const fn new(
        key: &'static str,
        description: &'static str,
        default: T::Default,
        rule: T::Rule,
        on_bad_row: Option<T::Default>,
    ) -> Self {
        Self {
            key,
            description,
            default,
            rule,
            on_bad_row,
            _type: PhantomData,
        }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub fn default(&self) -> T {
        T::from_default(&self.default)
    }
}

/// The type-erased view of a [`Setting`], used wherever all settings are
/// handled uniformly (pins, storage, schema, read APIs).
pub trait AnySetting: Send + Sync {
    /// Dotted key, e.g. `auth.session_ttl`.
    fn key(&self) -> &'static str;
    /// The first key segment, e.g. `auth`.
    fn category(&self) -> &'static str {
        let key = self.key();
        key.split_once('.').map_or(key, |(category, _)| category)
    }
    fn description(&self) -> &'static str;
    /// Sensitive values are encrypted at rest and never shown.
    fn is_sensitive(&self) -> bool;
    fn default_value(&self) -> Value;
    /// Effective value when a stored row exists but cannot be used.
    fn bad_row_value(&self) -> Value;
    /// Parses (native or text JSON) and checks the rule. The error never
    /// contains the input.
    fn coerce(&self, raw: &Json) -> Result<Value, String>;
    /// Exported schema entry.
    fn schema(&self) -> Json;
}

impl<T: SettingType> AnySetting for Setting<T> {
    fn key(&self) -> &'static str {
        self.key
    }

    fn description(&self) -> &'static str {
        self.description.trim()
    }

    fn is_sensitive(&self) -> bool {
        T::SENSITIVE
    }

    fn default_value(&self) -> Value {
        self.default().into_value()
    }

    fn bad_row_value(&self) -> Value {
        match &self.on_bad_row {
            Some(strict) => T::from_default(strict).into_value(),
            None => self.default_value(),
        }
    }

    fn coerce(&self, raw: &Json) -> Result<Value, String> {
        let value = T::parse(raw)?;
        value.check(&self.rule)?;
        Ok(value.into_value())
    }

    fn schema(&self) -> Json {
        let default = match T::SENSITIVE {
            true => Json::Null,
            false => self.default_value().to_json(),
        };
        let mut map = Map::from_iter([
            ("key".to_owned(), Json::from(self.key)),
            ("category".to_owned(), Json::from(self.category())),
            ("description".to_owned(), Json::from(self.description())),
            ("type".to_owned(), Json::from(T::TYPE_NAME)),
            ("sensitive".to_owned(), Json::Bool(T::SENSITIVE)),
            ("default".to_owned(), default),
        ]);
        map.extend(T::describe(&self.rule));
        if self.on_bad_row.is_some() {
            map.insert("on_bad_row".to_owned(), self.bad_row_value().to_json());
        }
        Json::Object(map)
    }
}

/// Declares settings and the [`all`](super::all) list in one place, so a
/// setting cannot exist without being registered. The doc comment is the
/// description; `rule` (default: the kind's rule) and `on_bad_row`
/// (default: the declared default) are optional, in that order.
macro_rules! settings {
    (@rule $ty:ty) => { <$ty as SettingType>::DEFAULT_RULE };
    (@rule $ty:ty, $rule:expr) => { $rule };
    (@bad_row) => { None };
    (@bad_row $policy:expr) => { $policy };
    ($(
        $(#[doc = $doc:literal])+
        $name:ident: $ty:ty = $key:literal {
            default: $default:expr
            $(, rule: $rule:expr)?
            $(, on_bad_row: $policy:expr)?
            $(,)?
        }
    )*) => {
        $(
            $(#[doc = $doc])+
            #[doc = concat!("\n\nKey: `", $key, "`.")]
            pub static $name: Setting<$ty> = Setting::new(
                $key,
                concat!($($doc),+),
                $default,
                settings!(@rule $ty $(, $rule)?),
                settings!(@bad_row $($policy)?),
            );
        )*

        static ALL: &[&dyn AnySetting] = &[$(&$name),*];
    };
}

pub(crate) use settings;
