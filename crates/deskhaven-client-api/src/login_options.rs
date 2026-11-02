//! `GET /api/login-options` (03-client-api-contract §3.1): third-party login buttons. The
//! Rust client also calls it as a TLS probe, so it takes no auth, touches nothing slow (the
//! buttons are cached by the OIDC service) and always answers 200.
//!
//! Flutter (`flutter/lib/models/user_model.dart:240-264`) takes the first
//! `common-oidc/<json [{name, icon}]>` item if there is one, else every `oidc/<name>`
//! item. Plain `oidc/<name>` items when no provider has an icon; otherwise one
//! `common-oidc/` item carrying every provider with its icon (an SVG document,
//! `login.dart:79-82`; `null` keeps the built-in one).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::oidc::LoginButton;
use deskhaven_core::state::AppState;
use serde_json::json;

const OIDC_PREFIX: &str = "oidc/";
const COMMON_OIDC_PREFIX: &str = "common-oidc/";

pub async fn login_options(State(state): State<Arc<AppState>>) -> Json<Vec<String>> {
    Json(options(&state.oidc.login_buttons().await))
}

fn options(buttons: &[LoginButton]) -> Vec<String> {
    if buttons.iter().all(|b| b.icon.is_none()) {
        return buttons
            .iter()
            .map(|b| format!("{OIDC_PREFIX}{}", b.name))
            .collect();
    }
    let list: Vec<_> = buttons
        .iter()
        .map(|b| json!({ "name": b.name, "icon": b.icon }))
        .collect();
    vec![format!("{COMMON_OIDC_PREFIX}{}", json!(list))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn button(name: &str, icon: Option<&str>) -> LoginButton {
        LoginButton {
            name: name.to_owned(),
            icon: icon.map(str::to_owned),
        }
    }

    #[test]
    fn plain_names_without_icons_and_one_common_item_with_any_icon() {
        assert!(options(&[]).is_empty());
        assert_eq!(
            options(&[button("azure", None), button("corp", None)]),
            ["oidc/azure", "oidc/corp"]
        );
        let common = options(&[button("azure", None), button("corp", Some("<svg/>"))]);
        assert_eq!(common.len(), 1);
        let list: serde_json::Value =
            serde_json::from_str(common[0].strip_prefix(COMMON_OIDC_PREFIX).unwrap()).unwrap();
        assert_eq!(
            list,
            json!([{"name": "azure", "icon": null}, {"name": "corp", "icon": "<svg/>"}])
        );
    }
}
