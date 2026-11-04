//! Query parameters every audit list takes (time window, order) and the checks on the
//! others (`device`, `q`, enum names).

use deskhaven_core::audit_query::{AuditQueryError, Listing};
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::domain_error::invalid_field;
use crate::{AdminApiError, PageQuery};

/// Longest accepted `q`.
pub(super) const MAX_Q_CHARS: usize = 200;
/// Longest accepted `device`, `result`, `action` and `target_type`.
pub(super) const MAX_LABEL_CHARS: usize = 256;

/// Time window and order, shared by every audit list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WindowQuery {
    /// Only records created at or after this time (unix milliseconds).
    #[garde(range(min = 0))]
    #[param(minimum = 0)]
    pub from: Option<i64>,
    /// Only records created before this time (unix milliseconds).
    #[garde(range(min = 0))]
    #[param(minimum = 0)]
    pub to: Option<i64>,
    /// `created_at` (oldest first) or `-created_at` (newest first, the default).
    #[garde(length(max = 16))]
    #[param(pattern = "^-?created_at$")]
    pub sort: Option<String>,
}

impl WindowQuery {
    /// The core listing for this window and `page`.
    pub(super) fn listing(&self, page: PageQuery) -> Result<Listing, AdminApiError> {
        if let (Some(from), Some(to)) = (self.from, self.to)
            && from > to
        {
            return Err(invalid_field("to", "must not be before from"));
        }
        let oldest_first = match self.sort.as_deref() {
            None | Some("-created_at") => false,
            Some("created_at") => true,
            Some(_) => return Err(invalid_field("sort", "unknown sort field")),
        };
        Ok(Listing {
            from: self.from,
            to: self.to,
            oldest_first,
            page: page.into(),
        })
    }
}

/// garde rule of the text filters: no control characters. A NUL makes PostgreSQL reject the
/// query (a 500), and no stored text has one (see `deskhaven_core::text`).
pub(super) fn plain_text(value: &Option<String>, _context: &()) -> garde::Result {
    match value {
        Some(text) if text.chars().any(char::is_control) => {
            Err(garde::Error::new("must not contain control characters"))
        }
        _ => Ok(()),
    }
}

/// `value` without surrounding blanks; `None` when nothing is left.
pub(super) fn trimmed(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// `name` as a variant of a database enum by its stored name (`parse` is
/// [`parse_stored`]); an unknown name is a validation error on `field`.
pub(super) fn known<E>(
    field: &str,
    name: Option<&str>,
    parse: fn(&str) -> Option<E>,
) -> Result<Option<E>, AdminApiError> {
    let Some(name) = name else { return Ok(None) };
    parse(name)
        .map(Some)
        .ok_or_else(|| invalid_field(field, "unknown value"))
}

impl From<AuditQueryError> for AdminApiError {
    fn from(err: AuditQueryError) -> Self {
        match err {
            AuditQueryError::Invalid { field, reason } => invalid_field(field, reason),
            other => Self::internal(other),
        }
    }
}
