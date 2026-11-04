// vendored-revision: fae47e0
// Source: https://github.com/rustdesk/hbb_common, commit fae47e0 (the submodule pin of
// rustdesk/rustdesk 23e5bb4), file src/config.rs. Trimmed to the two items
// `cargo xtask gen-strategy-catalog` parses (`option2bool` and the `keys` module), copied
// verbatim. License: AGPL-3.0, as part of RustDesk (hbb_common carries no separate licence
// file at this revision). Read as text only, never compiled. Refresh: xtask/src/catalog/mod.rs.

// This function must be kept the same as the one in flutter and sciter code.
// flutter: flutter/lib/common.dart -> option2bool()
// sciter: Does not have the function, but it should be kept the same.
pub fn option2bool(option: &str, value: &str) -> bool {
    if option.starts_with("enable-") {
        value != "N"
    } else if option.starts_with("allow-")
        || option == "stop-service"
        || option == keys::OPTION_DIRECT_SERVER
        || option == "force-always-relay"
    {
        value == "Y"
    } else {
        value != "N"
    }
}

pub mod keys {
    // Only the keys hbb_common itself references.
    pub const OPTION_COLLAPSE_TOOLBAR: &str = "collapse_toolbar";
    pub const OPTION_ZOOM_CURSOR: &str = "zoom-cursor";
    pub const OPTION_ENABLE_FILE_COPY_PASTE: &str = "enable-file-copy-paste";
    pub const OPTION_I444: &str = "i444";
    pub const OPTION_REVERSE_MOUSE_WHEEL: &str = "reverse_mouse_wheel";
    pub const OPTION_SWAP_LEFT_RIGHT_MOUSE: &str = "swap-left-right-mouse";
    pub const OPTION_DISPLAYS_AS_INDIVIDUAL_WINDOWS: &str = "displays_as_individual_windows";
    pub const OPTION_USE_ALL_MY_DISPLAYS_FOR_THE_REMOTE_SESSION: &str =
        "use_all_my_displays_for_the_remote_session";
    pub const OPTION_VIEW_STYLE: &str = "view_style";
    pub const OPTION_SCROLL_STYLE: &str = "scroll_style";
    pub const OPTION_EDGE_SCROLL_EDGE_THICKNESS: &str = "edge-scroll-edge-thickness";
    pub const OPTION_IMAGE_QUALITY: &str = "image_quality";
    pub const OPTION_CUSTOM_IMAGE_QUALITY: &str = "custom_image_quality";
    pub const OPTION_CUSTOM_FPS: &str = "custom-fps";
    pub const OPTION_CODEC_PREFERENCE: &str = "codec-preference";
    pub const OPTION_LANGUAGE: &str = "lang";
    pub const OPTION_ALLOW_NUMERNIC_ONE_TIME_PASSWORD: &str = "allow-numeric-one-time-password";
    pub const OPTION_DIRECT_SERVER: &str = "direct-server";
    pub const OPTION_ALLOW_WEBSOCKET: &str = "allow-websocket";
    pub const OPTION_WEBSOCKET_PORT: &str = "websocket-port";
    pub const OPTION_TRACKPAD_SPEED: &str = "trackpad-speed";
    pub const OPTION_REGISTER_DEVICE: &str = "register-device";
    pub const OPTION_RELAY_SERVER: &str = "relay-server";
    pub const OPTION_ICE_SERVERS: &str = "ice-servers";
    pub const OPTION_ALLOW_INSECURE_TLS_FALLBACK: &str = "allow-insecure-tls-fallback";
    pub const OPTION_ALLOW_WEBRTC_CC: &str = "allow-webrtc-congestion-control";
    pub const OPTION_ALLOW_HOSTNAME_AS_ID: &str = "allow-hostname-as-id";
    pub const OPTION_DISABLE_CHANGE_PERMANENT_PASSWORD: &str = "disable-change-permanent-password";
    pub const OPTION_DISABLE_CHANGE_ID: &str = "disable-change-id";
    pub const OPTION_DISABLE_UNLOCK_PIN: &str = "disable-unlock-pin";

    // proxy settings
    // The following options are not real keys, they are just used for custom client advanced settings.
    // The real keys are in Config2::socks.
    pub const OPTION_PROXY_URL: &str = "proxy-url";
    pub const OPTION_PROXY_USERNAME: &str = "proxy-username";
    pub const OPTION_PROXY_PASSWORD: &str = "proxy-password";
}
