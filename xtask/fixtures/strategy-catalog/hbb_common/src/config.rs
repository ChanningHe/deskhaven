// vendored-revision: fixture-hbb
// Trimmed fixture of rustdesk/hbb_common `src/config.rs`: only the shapes the strategy
// catalog generator reads (the `keys` module and `option2bool`).

pub const NOT_A_KEY: &str = "outside the keys module";

pub fn option2bool(option: &str, value: &str) -> bool {
    if option.starts_with("enable-") {
        value != "N"
    } else if option.starts_with("allow-")
        || option == "stop-service"
        || option == keys::OPTION_DIRECT_SERVER
    {
        value == "Y"
    } else {
        value != "N"
    }
}

pub mod keys {
    // Only the keys hbb_common itself references.
    pub const OPTION_DIRECT_SERVER: &str = "direct-server";
    pub const OPTION_ALLOW_WEBSOCKET: &str = "allow-websocket";
    pub const OPTION_DISABLE_CHANGE_ID: &str =
        "disable-change-id";

    // proxy settings
    pub const OPTION_PROXY_PASSWORD: &str = "proxy-password";
}

pub fn common_load() {}
