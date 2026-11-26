use std::marker::PhantomData;

use serde_json::{Map, Value as Json};

use super::kind::SettingType;
use super::value::Value;
use crate::bootstrap::{SETTINGS_KEY, env_var};

pub(crate) fn env_name(key: &str) -> String {
    env_var(&format!("{SETTINGS_KEY}.{key}"))
}

/// Declared only through `settings!`; read with [`Snapshot::get`](super::Snapshot::get).
pub struct Setting<T: SettingType> {
    key: &'static str,
    description: &'static str,
    default: T::Default,
    rule: T::Rule,
    /// Used when a stored row cannot be used; `None` = the default. Set for security-relevant
    /// settings, so corruption or a lost master key fails closed rather than open.
    on_bad_row: Option<T::Default>,
    /// Only a pin sets the value: a stored row is ignored and every write is
    /// refused, so the console cannot turn it on.
    pin_only: bool,
    _type: PhantomData<fn() -> T>,
}

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
        pin_only: bool,
    ) -> Self {
        Self {
            key,
            description,
            default,
            rule,
            on_bad_row,
            pin_only,
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

pub trait AnySetting: Send + Sync {
    fn key(&self) -> &'static str;
    fn category(&self) -> &'static str {
        let key = self.key();
        key.split_once('.').map_or(key, |(category, _)| category)
    }
    fn description(&self) -> &'static str;
    /// Sensitive values are encrypted at rest and never shown.
    fn is_sensitive(&self) -> bool;
    fn is_pin_only(&self) -> bool;
    fn default_value(&self) -> Value;
    fn bad_row_value(&self) -> Value;
    /// The error never contains the input.
    fn coerce(&self, raw: &Json) -> Result<Value, String>;
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

    fn is_pin_only(&self) -> bool {
        self.pin_only
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
            ("env".to_owned(), Json::from(env_name(self.key))),
            ("pin_only".to_owned(), Json::Bool(self.pin_only)),
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
/// description; `rule` (default: the kind's rule), `on_bad_row` (default:
/// the declared default) and `pin_only` (default: false) are optional, in
/// that order.
macro_rules! settings {
    (@rule $ty:ty) => { <$ty as SettingType>::DEFAULT_RULE };
    (@rule $ty:ty, $rule:expr) => { $rule };
    (@bad_row) => { None };
    (@bad_row $policy:expr) => { $policy };
    (@pin_only) => { false };
    (@pin_only $pin_only:literal) => { $pin_only };
    ($(
        $(#[doc = $doc:literal])+
        $name:ident: $ty:ty = $key:literal {
            default: $default:expr
            $(, rule: $rule:expr)?
            $(, on_bad_row: $policy:expr)?
            $(, pin_only: $pin_only:literal)?
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
                settings!(@pin_only $($pin_only)?),
            );
        )*

        static ALL: &[&dyn AnySetting] = &[$(&$name),*];
    };
}

pub(crate) use settings;
