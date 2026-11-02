//! Admin HTTP API (`/admin/api/v1`) backing the web console.

mod error;
mod stack_error;

pub use error::{AdminApiError, BoxError, ErrorCode};
pub use stack_error::render_stack_error;
