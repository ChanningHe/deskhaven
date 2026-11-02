//! Shared HTTP plumbing for the client and admin APIs: real client IP,
//! middleware stack, health routes and listeners.

mod error;
mod health;
mod middleware;
mod real_ip;
mod request_id;
mod serve;

pub use error::error_chain;
pub use health::{NotReady, ReadinessCheck, health_routes};
pub use middleware::{
    DEFAULT_BODY_LIMIT, DEFAULT_REQUEST_TIMEOUT, HttpConfig, INTERNAL_ERROR_BODY, apply_middleware,
};
pub use real_ip::{ClientIp, ClientIpRejection, TrustedProxies, X_FORWARDED_FOR};
pub use request_id::X_REQUEST_ID;
pub use serve::{Apps, ListenConfig, Listeners, ServeError};
