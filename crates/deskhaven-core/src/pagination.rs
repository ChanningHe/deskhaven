//! Offset pagination shared by list operations.

/// Largest accepted page size.
pub const MAX_PAGE_SIZE: u64 = 100;
/// Largest accepted page number. Keeps the offset (`(page - 1) * page_size` <= 10^8) far
/// from any integer limit of Rust or either database; nobody pages that deep.
pub const MAX_PAGE: u64 = 1_000_000;
const DEFAULT_PAGE_SIZE: u64 = 20;

/// A 1-based page of `page_size` items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageRequest {
    pub page: u64,
    pub page_size: u64,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            page: 1,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// Why a [`PageRequest`] is unusable: the offending field and the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageProblem {
    pub field: &'static str,
    pub reason: String,
}

impl PageRequest {
    /// The zero-based page index, or why the request is out of range.
    pub fn index(&self) -> Result<u64, PageProblem> {
        if self.page == 0 || self.page > MAX_PAGE {
            return Err(PageProblem {
                field: "page",
                reason: format!("must be between 1 and {MAX_PAGE}"),
            });
        }
        if self.page_size == 0 || self.page_size > MAX_PAGE_SIZE {
            return Err(PageProblem {
                field: "page_size",
                reason: format!("must be between 1 and {MAX_PAGE_SIZE}"),
            });
        }
        Ok(self.page - 1)
    }
}

/// One page of results with the total number of matching items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(page: u64, page_size: u64) -> PageRequest {
        PageRequest { page, page_size }
    }

    #[test]
    fn index_is_zero_based_within_bounds() {
        assert_eq!(request(1, 1).index(), Ok(0));
        assert_eq!(request(MAX_PAGE, MAX_PAGE_SIZE).index(), Ok(MAX_PAGE - 1));
    }

    #[test]
    fn out_of_range_pages_are_problems_not_overflows() {
        for page in [0, MAX_PAGE + 1, 100_000_000_000_000_000, u64::MAX] {
            let problem = request(page, MAX_PAGE_SIZE).index().err();
            assert_eq!(problem.map(|p| p.field), Some("page"), "{page}");
        }
        for size in [0, MAX_PAGE_SIZE + 1] {
            let problem = request(1, size).index().err();
            assert_eq!(problem.map(|p| p.field), Some("page_size"), "{size}");
        }
    }
}
