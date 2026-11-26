//! Pure: no database (persistence lives in `deskhaven-core::settings`).
//! Effective value precedence: pin > database > default.

mod def;
mod enums;
mod ice;
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
pub use enums::{AcceptPresets, AclAction, LinkEmailMaxRole, Registration, SmtpTls, TfaEnforce};
pub use ice::{MAX_ICE_SERVERS, ice_urls, is_turn_url};
pub use kind::{Bound, ListRule, Range, SettingType};
pub use pins::{PinError, PinProblem, PinSet};
pub use registry::*;
pub use snapshot::{Entry, LoadWarning, SettingView, Snapshot, Stored};
pub use text::TextRule;
pub use update::{
    Accepted, Change, Problem, Rejection, Reset, check_rules, prepare, prepare_batch,
};
pub use value::{HostPort, HostPortError, Value};

pub fn schema() -> Json {
    let settings: Vec<Json> = all().iter().map(|s| s.schema()).collect();
    json!({ "settings": settings, "rules": rules::schema() })
}
