//! Success bodies the Rust client reads through `HbbHttpResponse::parse`.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

const ERROR_KEY: &str = "error";

/// Any `error` key, whatever its value, makes the client take the body as an error and never
/// read the data, so a success body must not carry one.
#[derive(Debug)]
pub struct HbbData<T>(pub T);

impl<'de, T: DeserializeOwned> Deserialize<'de> for HbbData<T> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let map = Map::<String, Value>::deserialize(de)?;
        if map.contains_key(ERROR_KEY) {
            return Err(D::Error::custom(
                "a top-level \"error\" key makes HbbHttpResponse an error, the data is not read",
            ));
        }
        T::deserialize(Value::Object(map))
            .map(Self)
            .map_err(D::Error::custom)
    }
}
