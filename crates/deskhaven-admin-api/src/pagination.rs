//! List pagination (08 §6): `?page=&page_size=` in, `{items, total, page, page_size}` out.
//! Bounds are core's ([`MAX_PAGE`], [`MAX_PAGE_SIZE`] = 200); a list handler takes
//! `ValidQuery<PageQuery>` next to its own filter query.

use deskhaven_core::pagination::{MAX_PAGE, MAX_PAGE_SIZE, Page, PageRequest};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// `page` (1-based, default 1) and `page_size` (default 20, at most 200).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    #[serde(default = "default_page")]
    #[garde(range(min = 1, max = MAX_PAGE))]
    #[param(minimum = 1, maximum = 1_000_000, default = 1)]
    pub page: u64,
    #[serde(default = "default_page_size")]
    #[garde(range(min = 1, max = MAX_PAGE_SIZE))]
    #[param(minimum = 1, maximum = 200, default = 20)]
    pub page_size: u64,
}

fn default_page() -> u64 {
    PageRequest::default().page
}

fn default_page_size() -> u64 {
    PageRequest::default().page_size
}

impl Default for PageQuery {
    fn default() -> Self {
        Self {
            page: default_page(),
            page_size: default_page_size(),
        }
    }
}

impl From<PageQuery> for PageRequest {
    fn from(q: PageQuery) -> Self {
        Self {
            page: q.page,
            page_size: q.page_size,
        }
    }
}

/// One page of a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Paginated<T> {
    pub items: Vec<T>,
    /// Matching items across all pages.
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

impl<T> Paginated<T> {
    /// A core page with each item converted to its wire type.
    pub fn from_page<U>(page: Page<U>, convert: impl FnMut(U) -> T) -> Self {
        Self {
            items: page.items.into_iter().map(convert).collect(),
            total: page.total,
            page: page.page,
            page_size: page.page_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_bounds_match_core() {
        assert_eq!(MAX_PAGE, 1_000_000);
        assert_eq!(MAX_PAGE_SIZE, 200);
        assert_eq!(
            PageQuery::default(),
            PageQuery {
                page: 1,
                page_size: 20
            }
        );
    }

    #[test]
    fn rejects_out_of_range_values() {
        let bad = [
            PageQuery {
                page: 0,
                page_size: 20,
            },
            PageQuery {
                page: MAX_PAGE + 1,
                page_size: 20,
            },
            PageQuery {
                page: 1,
                page_size: 0,
            },
            PageQuery {
                page: 1,
                page_size: MAX_PAGE_SIZE + 1,
            },
        ];
        for q in bad {
            assert!(q.validate().is_err(), "{q:?}");
        }
        assert!(
            PageQuery {
                page: MAX_PAGE,
                page_size: MAX_PAGE_SIZE
            }
            .validate()
            .is_ok()
        );
    }
}
