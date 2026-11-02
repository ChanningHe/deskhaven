//! Shared HTTP plumbing for the client and admin APIs: real client IP,
//! middleware stack, stack-error rendering, health routes and listeners.

mod conn;
mod error;
mod health;
mod middleware;
mod paths;
mod real_ip;
mod request_id;
mod serve;
mod stack_error;

pub use error::error_chain;
pub use health::{NotReady, ReadinessCheck, health_routes};
pub use middleware::{
    DEFAULT_BODY_LIMIT, DEFAULT_DRAIN_TIMEOUT, DEFAULT_HEADER_READ_TIMEOUT,
    DEFAULT_MAX_CONNECTIONS, DEFAULT_REQUEST_TIMEOUT, HttpConfig, INTERNAL_ERROR_BODY,
    apply_middleware,
};
pub use paths::{ADMIN_API_PREFIX, CLIENT_API_PREFIX};
pub use real_ip::{ClientIp, ClientIpRejection, InternalPeer, TrustedProxies, X_FORWARDED_FOR};
pub use request_id::X_REQUEST_ID;
pub use serve::{Apps, ListenConfig, Listeners, ServeError};
pub use stack_error::{ErrorBodies, ErrorRenderer, StackError, plain_text};
