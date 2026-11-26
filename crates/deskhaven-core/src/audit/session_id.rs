use serde_json::Value;

pub(super) const UNBOUND: &str = "0";

/// `123`, `"123"` and `"0123"` are the same session. The opening record is sent before the
/// login request, so it carries 0, the unbound session id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn from_json(value: &Value) -> Option<Self> {
        let number = match value {
            Value::Number(n) => n
                .as_u64()
                .map(i128::from)
                .or_else(|| n.as_i64().map(i128::from))?,
            Value::String(s) => s.parse::<i128>().ok()?,
            _ => return None,
        };
        let in_range = (i128::from(i64::MIN)..=i128::from(u64::MAX)).contains(&number);
        in_range.then(|| Self(number.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_unbound(&self) -> bool {
        self.0 == UNBOUND
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> Option<String> {
        SessionId::from_json(&value).map(|id| id.0)
    }

    #[test]
    fn numbers_and_decimal_strings_normalize_to_the_same_text() {
        let big = 18_446_744_073_709_551_615_u64;

        assert_eq!(parse(json!(123)), Some("123".to_owned()));
        assert_eq!(parse(json!("123")), Some("123".to_owned()));
        assert_eq!(parse(json!("0123")), Some("123".to_owned()));
        assert_eq!(parse(json!("+123")), Some("123".to_owned()));
        assert_eq!(parse(json!(big)), Some(big.to_string()));
        assert_eq!(parse(json!(big.to_string())), Some(big.to_string()));
        assert_eq!(parse(json!(-5)), Some("-5".to_owned()));
    }

    #[test]
    fn other_values_are_refused() {
        let too_big = (i128::from(u64::MAX) + 1).to_string();
        let too_small = (i128::from(i64::MIN) - 1).to_string();

        for value in [
            json!(1.5),
            json!("1.5"),
            json!(""),
            json!(" 1"),
            json!("abc"),
            json!(too_big),
            json!(too_small),
            json!(null),
            json!(true),
            json!([1]),
        ] {
            assert_eq!(parse(value.clone()), None, "{value}");
        }
    }

    #[test]
    fn zero_is_unbound() {
        let zero = SessionId::from_json(&json!("00")).unwrap();
        let other = SessionId::from_json(&json!(7)).unwrap();

        assert!(zero.is_unbound());
        assert!(!other.is_unbound());
    }
}
