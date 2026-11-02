//! Success bodies the Rust client reads through `HbbHttpResponse::parse`.

use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

/// Name of the key that turns any body into an error.
const ERROR_KEY: &str = "error";

/// `src/hbbs_http.rs:24-39`: the body must be a JSON object. If it has an
/// `error` key, whatever its value (`""`, `null`, ...), the client takes it
/// as an error (`Error` for a string, else `ErrorFormat`) and never reads the
/// data. Otherwise the whole object must deserialize as `T`, else
/// `DataTypeFormat`. So a success body must not carry `error` at all.
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, Deserialize)]
    struct Data {
        code: String,
    }

    fn parse(value: Value) -> Result<HbbData<Data>, serde_json::Error> {
        serde_json::from_value(value)
    }

    #[test]
    fn data_without_error_parses() {
        assert_eq!(parse(json!({ "code": "c1", "x": 1 })).unwrap().0.code, "c1");
    }

    #[test]
    fn any_error_key_rejects_the_body() {
        assert!(parse(json!({ "code": "c1", "error": "" })).is_err());
        assert!(parse(json!({ "code": "c1", "error": null })).is_err());
    }

    #[test]
    fn non_objects_and_bad_data_are_rejected() {
        assert!(parse(json!(["c1"])).is_err());
        assert!(parse(json!({ "code": 1 })).is_err());
    }
}
