//! The stored names of the database enums the audit tables use (`send`, `oidc`,
//! `api_token`, ...): the strings the API shows and accepts.

use sea_orm::ActiveEnum;
use sea_orm::strum::IntoEnumIterator;

/// The name `variant` is stored under.
pub fn stored_name<E: ActiveEnum<Value = String>>(variant: &E) -> String {
    variant.to_value()
}

/// The variant stored under `name`, if there is one.
pub fn parse_stored<E>(name: &str) -> Option<E>
where
    E: ActiveEnum<Value = String> + IntoEnumIterator,
{
    E::iter().find(|variant| variant.to_value() == name)
}

#[cfg(test)]
mod tests {
    use deskhaven_db::entity::login_event::Method;

    use super::*;

    #[test]
    fn names_round_trip_and_unknown_names_are_none() {
        assert_eq!(stored_name(&Method::EmailCode), "email_code");
        assert_eq!(
            parse_stored::<Method>("email_code"),
            Some(Method::EmailCode)
        );
        assert_eq!(parse_stored::<Method>("Email_Code"), None);
        assert_eq!(parse_stored::<Method>(""), None);
    }
}
