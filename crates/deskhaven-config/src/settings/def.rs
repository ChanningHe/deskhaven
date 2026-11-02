//! One setting declaration: key, description, typed default and rule.

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
    _type: PhantomData<fn() -> T>,
}

impl<T: SettingType> Setting<T> {
    pub(crate) const fn new(
        key: &'static str,
        description: &'static str,
        default: T::Default,
        rule: T::Rule,
    ) -> Self {
        Self {
            key,
            description,
            default,
            rule,
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
        Json::Object(map)
    }
}

/// Declares settings and the [`all`](super::all) list in one place, so a
/// setting cannot exist without being registered. The doc comment is the
/// description; `rule` is optional (the kind's default rule applies).
macro_rules! settings {
    (@rule $ty:ty) => { <$ty as SettingType>::DEFAULT_RULE };
    (@rule $ty:ty, $rule:expr) => { $rule };
    ($(
        $(#[doc = $doc:literal])+
        $name:ident: $ty:ty = $key:literal {
            default: $default:expr
            $(, rule: $rule:expr)?
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
            );
        )*

        static ALL: &[&dyn AnySetting] = &[$(&$name),*];
    };
}

pub(crate) use settings;
