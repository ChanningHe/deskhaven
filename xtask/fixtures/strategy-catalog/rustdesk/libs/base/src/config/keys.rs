// vendored-revision: fixture-rustdesk
//! Trimmed fixture of rustdesk/rustdesk `libs/base/src/config/keys.rs`.

pub use hbb_common::config::keys::*;

pub const OPTION_ENABLE_KEYBOARD: &str = "enable-keyboard";
pub const OPTION_APPROVE_MODE: &str = "approve-mode";
pub const OPTION_WHITELIST: &str = "whitelist";
pub const OPTION_DISABLE_UDP: &str = "disable-udp";
pub const OPTION_THEME: &str = "theme";
pub const OPTION_HIDE_ELEVATE_BUTTON_IN_ACCEPT_WINDOW: &str =
    "hide-elevate-button-in-accept-window";

// DEFAULT_LOCAL_SETTINGS, OVERWRITE_LOCAL_SETTINGS
pub const KEYS_LOCAL_SETTINGS: &[&str] = &[OPTION_THEME];
// DEFAULT_SETTINGS, OVERWRITE_SETTINGS
pub const KEYS_SETTINGS: &[&str] = &[
    OPTION_ENABLE_KEYBOARD,
    OPTION_APPROVE_MODE,
    // Comments inside the list are skipped.
    OPTION_DIRECT_SERVER,
    OPTION_WHITELIST,
    OPTION_PROXY_PASSWORD,
    OPTION_ALLOW_WEBSOCKET,
    OPTION_DISABLE_UDP,
];
