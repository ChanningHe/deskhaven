use std::collections::HashMap;

use serde::Deserialize;

/// Each item is cast `as String` (a non-string throws); items are `oidc/<name>` or
/// `common-oidc/<json>`.
pub type LoginOptions = Vec<String>;

/// Only 0, 1 and -1 parse; missing means `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(try_from = "i64")]
pub enum ClientUserStatus {
    Disabled,
    #[default]
    Normal,
    Unverified,
}

impl TryFrom<i64> for ClientUserStatus {
    type Error = String;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Disabled),
            1 => Ok(Self::Normal),
            -1 => Ok(Self::Unverified),
            other => Err(format!("user status must be 0, 1 or -1, got {other}")),
        }
    }
}

/// Every field defaults to `""`.
#[derive(Debug, Deserialize)]
pub struct DeviceInfo {
    #[serde(default)]
    pub os: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
}

/// No field has a default.
#[derive(Debug, Deserialize)]
pub struct WhitelistItem {
    pub data: String,
    pub info: DeviceInfo,
    pub exp: u64,
}

/// `UserSettings` is flattened in. Present-but-`null` fails: serde defaults only cover a
/// missing key.
#[derive(Debug, Deserialize)]
pub struct UserInfo {
    #[serde(default)]
    pub email_verification: bool,
    #[serde(default)]
    pub email_alarm_notification: bool,
    #[serde(default)]
    pub login_device_whitelist: Vec<WhitelistItem>,
    #[serde(default)]
    pub other: HashMap<String, String>,
}

/// `info` is required on purpose although Flutter never reads it: the same payload reaches the
/// Rust client in the OIDC `AuthBody`, where a missing `info` fails the whole login.
#[derive(Debug, Deserialize)]
pub struct ClientUserPayload {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub status: ClientUserStatus,
    pub info: UserInfo,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub third_auth_type: Option<String>,
    #[serde(default)]
    pub verifier: Option<String>,
}

/// Every key is optional in Flutter; stricter here: `user` must be a full
/// [`ClientUserPayload`], the record the Rust client reads elsewhere.
#[derive(Debug, Deserialize)]
pub struct LoginResponse {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub tfa_type: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub user: Option<ClientUserPayload>,
}

/// As the login dialog uses it: `type` must be `email_check`, `tfa_type` picks the dialog, and
/// the second request sends `secret` back with `user.name` as the username.
#[derive(Debug, Deserialize)]
pub struct SecondFactorChallenge {
    #[serde(rename = "type")]
    pub kind: String,
    pub tfa_type: String,
    pub secret: String,
    pub user: ChallengeUser,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeUser {
    pub name: String,
}

/// `url` must be an absolute URL; read through `HbbHttpResponse`, hence `HbbData<OidcAuthUrl>`.
#[derive(Debug, Deserialize)]
pub struct OidcAuthUrl {
    pub code: String,
    pub url: url::Url,
}

/// `tfa_type` and `secret` default when missing, but a `null` fails the whole body (the client
/// then polls until it times out).
#[derive(Debug, Deserialize)]
pub struct AuthBody {
    pub access_token: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub tfa_type: String,
    #[serde(default)]
    pub secret: String,
    pub user: ClientUserPayload,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn user() -> Value {
        json!({
            "name": "alice", "display_name": "Alice", "avatar": "/avatars/1.png",
            "email": "a@x", "note": "", "status": 1, "is_admin": false,
            "info": {
                "email_verification": false, "email_alarm_notification": false,
                "login_device_whitelist": [], "other": {}
            },
            "third_auth_type": null, "verifier": null
        })
    }

    fn parses<T: serde::de::DeserializeOwned>(value: Value) -> bool {
        serde_json::from_value::<T>(value).is_ok()
    }

    #[test]
    fn login_options_are_strings() {
        assert!(parses::<LoginOptions>(json!([])));
        assert!(parses::<LoginOptions>(json!(["oidc/google"])));
        assert!(!parses::<LoginOptions>(json!([{ "name": "google" }])));
        assert!(!parses::<LoginOptions>(json!({})));
    }

    #[test]
    fn user_payload_from_the_contract_example() {
        let user: ClientUserPayload = serde_json::from_value(user()).unwrap();
        assert_eq!(user.status, ClientUserStatus::Normal);
        assert!(parses::<ClientUserPayload>(
            json!({ "name": "bob", "info": {}, "extra": 1 })
        ));
    }

    #[test]
    fn user_payload_rejects_what_the_rust_client_rejects() {
        let mut no_info = user();
        no_info.as_object_mut().unwrap().remove("info");
        let mut bad_status = user();
        bad_status["status"] = json!(2);
        let mut null_admin = user();
        null_admin["is_admin"] = Value::Null;

        assert!(!parses::<ClientUserPayload>(no_info));
        assert!(!parses::<ClientUserPayload>(bad_status));
        assert!(!parses::<ClientUserPayload>(null_admin));
    }

    #[test]
    fn statuses_map_like_the_client() {
        let status = |v: i64| ClientUserStatus::try_from(v);
        assert_eq!(status(0), Ok(ClientUserStatus::Disabled));
        assert_eq!(status(-1), Ok(ClientUserStatus::Unverified));
        assert!(status(3).is_err());
    }

    #[test]
    fn login_response_allows_the_two_step_challenge() {
        let challenge = json!({ "type": "email_check", "tfa_type": "tfa_check", "secret": "c1" });
        assert!(parses::<LoginResponse>(challenge));
        assert!(parses::<LoginResponse>(
            json!({ "type": "access_token", "access_token": "t", "user": user() })
        ));
    }

    #[test]
    fn second_factor_challenge_needs_secret_and_user_name() {
        let ok = json!({
            "type": "email_check", "tfa_type": "tfa_check", "secret": "c1",
            "user": { "name": "alice" }
        });
        let no_user = json!({ "type": "email_check", "tfa_type": "tfa_check", "secret": "c1" });
        let no_secret =
            json!({ "type": "email_check", "tfa_type": "tfa_check", "user": { "name": "a" } });
        let null_tfa = json!({ "type": "email_check", "tfa_type": null, "secret": "c1", "user": { "name": "a" } });

        assert!(parses::<SecondFactorChallenge>(ok));
        assert!(!parses::<SecondFactorChallenge>(no_user));
        assert!(!parses::<SecondFactorChallenge>(no_secret));
        assert!(!parses::<SecondFactorChallenge>(null_tfa));
    }

    #[test]
    fn auth_body_rejects_null_tfa_type_and_missing_user() {
        let ok = json!({ "access_token": "t", "type": "access_token", "user": user() });
        let null_tfa = json!({ "access_token": "t", "type": "access_token", "tfa_type": null, "user": user() });
        let no_user = json!({ "access_token": "t", "type": "access_token" });

        assert!(parses::<AuthBody>(ok));
        assert!(!parses::<AuthBody>(null_tfa));
        assert!(!parses::<AuthBody>(no_user));
    }
}
