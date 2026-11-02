//! Offset pagination shared by list operations.

/// Largest accepted page size.
pub const MAX_PAGE_SIZE: u64 = 100;
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
        if self.page == 0 {
            return Err(PageProblem {
                field: "page",
                reason: "must be at least 1".to_owned(),
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
