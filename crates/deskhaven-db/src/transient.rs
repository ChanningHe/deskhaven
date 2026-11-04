//! Failures that a retry of the same operation can cure: PostgreSQL aborts one of two
//! transactions that wait for each other's locks (`deadlock_detected`) and may refuse a
//! transaction that cannot be serialized (`serialization_failure`). Nothing was written.

use std::error::Error;

use sea_orm::DbErr;

use crate::constraint::database_code;

/// PostgreSQL `deadlock_detected`.
const PG_DEADLOCK: &str = "40P01";
/// PostgreSQL `serialization_failure`.
const PG_SERIALIZATION_FAILURE: &str = "40001";

/// Whether `err` is a deadlock or serialization failure reported by the database.
pub fn is_transient_conflict(err: &DbErr) -> bool {
    database_code(err).is_some_and(|code| code == PG_DEADLOCK || code == PG_SERIALIZATION_FAILURE)
}

/// Whether `err`, or anything in its `source()` chain, is a [`DbErr`] for which
/// [`is_transient_conflict`] holds (services wrap the database error in their own type).
pub fn chain_has_transient_conflict(err: &(dyn Error + 'static)) -> bool {
    let mut current = Some(err);
    while let Some(error) = current {
        if error
            .downcast_ref::<DbErr>()
            .is_some_and(is_transient_conflict)
        {
            return true;
        }
        current = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coded_error::coded_error;

    #[derive(Debug, thiserror::Error)]
    #[error("service failed")]
    struct Wrapped(#[source] DbErr);

    #[test]
    fn deadlocks_and_serialization_failures_are_transient() {
        assert!(is_transient_conflict(&coded_error("40P01")));
        assert!(is_transient_conflict(&coded_error("40001")));
    }

    #[test]
    fn other_database_errors_are_not() {
        assert!(!is_transient_conflict(&coded_error("23505")));
        assert!(!is_transient_conflict(&DbErr::Custom("40P01".to_owned())));
    }

    #[test]
    fn a_wrapped_database_error_is_found_through_the_source_chain() {
        assert!(chain_has_transient_conflict(&Wrapped(coded_error("40P01"))));
        assert!(!chain_has_transient_conflict(&Wrapped(coded_error(
            "23505"
        ))));
    }
}
