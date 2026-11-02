//! HTTP API consumed by the official RustDesk client (`/api/*`).

mod error;
mod stack_error;

pub use error::{BoxError, ClientApiError};
pub use stack_error::render_stack_error;
