//! Query strings of the paged endpoints: `current` (1-based), `pageSize`, for address book
//! peers `ab` (`flutter/lib/models/ab_model.dart:303-314, 1440-1451`) and for the group
//! panel `status` (`flutter/lib/models/group_model.dart:177-186`), and the `{total, data}`
//! reply. Values are taken verbatim: the client sends digits and UUIDs, which Dart's `Uri`
//! never percent-encodes; an encoded value simply matches nothing.

use deskhaven_core::pagination::PageRequest;
use serde::Serialize;

use crate::ClientApiError;

/// `{total, data}` (03 §1.3).
#[derive(Serialize)]
pub struct PageReply<T> {
    pub total: u64,
    pub data: Vec<T>,
}

const CURRENT: &str = "current";
const PAGE_SIZE: &str = "pageSize";
const BOOK: &str = "ab";
const STATUS: &str = "status";

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

/// The `status` filter (`1` normal, `0` disabled, `-1` unverified); a blank or missing value
/// is no filter. What the numbers mean for each endpoint is the caller's to check.
pub fn status(query: Option<&str>) -> Result<Option<i8>, ClientApiError> {
    match value(query.unwrap_or_default(), STATUS) {
        None | Some("") => Ok(None),
        Some(raw) => raw
            .parse::<i8>()
            .map(Some)
            .map_err(|_| ClientApiError::BadRequest("Invalid status".into())),
    }
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
    fn status_is_an_optional_small_integer() {
        assert_eq!(status(None).unwrap(), None);
        assert_eq!(status(Some("current=1&status=")).unwrap(), None);
        assert_eq!(status(Some("status=1")).unwrap(), Some(1));
        assert_eq!(status(Some("status=-1")).unwrap(), Some(-1));
        assert!(status(Some("status=x")).is_err());
        assert!(status(Some("status=1000")).is_err());
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
