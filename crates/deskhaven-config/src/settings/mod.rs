//! Runtime settings (05-config §3): the registry, value kinds, validation,
//! pin merging, snapshot building and schema export. Pure: no database.
//!
//! - Declaration: `registry.rs`, one `settings!` entry per key.
//! - Typed read: `snapshot.get(&settings::AUTH_SESSION_TTL)` returns a `Duration`.
//! - Effective value precedence: pin > database > default, each with its [`Source`](crate::Source).
//! - Updates: [`prepare`] (keys, pins, values) then [`check_rules`] on the
//!   candidate [`Snapshot`]; persistence lives in `deskhaven-core::settings`.

mod def;
mod enums;
mod kind;
mod pins;
mod registry;
mod rules;
mod snapshot;
mod text;
mod update;
mod value;

use serde_json::{Value as Json, json};

pub use def::{AnySetting, Setting};
pub use enums::{AcceptPresets, AclAction, Registration, SmtpTls, TfaEnforce};
pub use kind::{Bound, ListRule, Range, SettingType};
pub use pins::{PinError, PinProblem, PinSet};
pub use registry::*;
pub use snapshot::{Entry, LoadWarning, SettingView, Snapshot, Stored};
pub use text::TextRule;
pub use update::{
    Accepted, Change, Problem, Rejection, Reset, check_rules, prepare, prepare_batch,
};
pub use value::{HostPort, HostPortError, Value};

/// Machine-readable description of every setting and cross-key rule, for
/// the admin API, settings forms and generated docs.
pub fn schema() -> Json {
    let settings: Vec<Json> = all().iter().map(|s| s.schema()).collect();
    json!({ "settings": settings, "rules": rules::schema() })
}
