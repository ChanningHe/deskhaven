//! Query strings of the paged endpoints: `current` (1-based), `pageSize` and, for peers,
//! `ab` (`flutter/lib/models/ab_model.dart:303-314, 1440-1451`). Values are taken
//! verbatim: the client sends digits and UUIDs, which Dart's `Uri` never percent-encodes;
//! an encoded value simply matches nothing.

use deskhaven_core::pagination::PageRequest;

use crate::ClientApiError;

const CURRENT: &str = "current";
const PAGE_SIZE: &str = "pageSize";
const BOOK: &str = "ab";

/// The value of the first `key=value` pair named `key`.
fn value<'q>(query: &'q str, key: &str) -> Option<&'q str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// The page asked for; a missing parameter takes the default. Range checks are the
/// service's ([`PageRequest::index`]).
pub fn page(query: Option<&str>) -> Result<PageRequest, ClientApiError> {
    let query = query.unwrap_or_default();
    let defaults = PageRequest::default();
    let number = |key: &str, default: u64| match value(query, key) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<u64>()
            .map_err(|_| ClientApiError::BadRequest(format!("Invalid {key}").into())),
    };
    Ok(PageRequest {
        page: number(CURRENT, defaults.page)?,
        page_size: number(PAGE_SIZE, defaults.page_size)?,
    })
}

/// The `ab` guid, required.
pub fn book(query: Option<&str>) -> Result<&str, ClientApiError> {
    value(query.unwrap_or_default(), BOOK)
        .filter(|guid| !guid.is_empty())
        .ok_or_else(|| ClientApiError::BadRequest("Missing ab".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_client_query() {
        let query = Some("current=2&pageSize=100&ab=g-1");

        let request = page(query).unwrap();

        assert_eq!((request.page, request.page_size), (2, 100));
        assert_eq!(book(query).unwrap(), "g-1");
    }

    #[test]
    fn missing_values_default_and_bad_ones_are_refused() {
        let defaults = PageRequest::default();

        assert_eq!(page(None).unwrap(), defaults);
        assert!(page(Some("current=x")).is_err());
        assert!(page(Some("pageSize=-1")).is_err());
        assert!(book(Some("current=1")).is_err());
        assert!(book(Some("ab=")).is_err());
    }
}
