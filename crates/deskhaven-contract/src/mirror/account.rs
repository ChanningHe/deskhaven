//! Account endpoints (03 §3): login options, login, current user, OIDC.

use std::collections::HashMap;

use serde::Deserialize;

/// `GET /api/login-options` (03 §3.1).
///
/// `flutter/lib/models/user_model.dart:240-264`: `jsonDecode(resp.body)` is
/// iterated and each item cast `as String` (a non-string throws); any non-2xx
/// status throws. Items are `oidc/<name>` or `common-oidc/<json>`.
pub type LoginOptions = Vec<String>;

/// User status as an integer.
///
/// `src/hbbs_http/account.rs:71-77` (`Deserialize_repr`, `i64`: only 0, 1
/// and -1 parse) and `:129-133` (missing means `Normal`);
/// `flutter/lib/common/hbbs/hbbs.dart:43-47`.
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

/// `deviceInfo` of a whitelist entry.
///
/// `src/hbbs_http/account.rs:30-44`: every field defaults to `""`.
#[derive(Debug, Deserialize)]
pub struct DeviceInfo {
    #[serde(default)]
    pub os: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
}

/// `src/hbbs_http/account.rs:46-51`: no field has a default.
#[derive(Debug, Deserialize)]
pub struct WhitelistItem {
    pub data: String,
    pub info: DeviceInfo,
    pub exp: u64,
}

/// `user.info`.
///
/// `src/hbbs_http/account.rs:53-69` (`UserSettings` is flattened in, so its
/// two flags sit next to the other keys). Present-but-`null` fails: serde
/// defaults only cover a missing key.
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

/// UserPayload: `currentUser`, `user` of an auth body, items of `/api/users`.
///
/// The Rust client is the strict reader: `src/hbbs_http/account.rs:79-97`
/// (`name` and `info` required). The Flutter reader,
/// `flutter/lib/common/hbbs/hbbs.dart:36-48`, adds `verifier` (`String?`).
///
/// `info` is required on purpose, although Flutter never reads it (and
/// defaults `name` to `''`): the same payload reaches the Rust client in the
/// OIDC `AuthBody`, where a missing `info` fails the whole login. Requiring
/// it everywhere keeps one server shape for every reader.
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

/// `POST /api/login` responses, read by Flutter (03 §3.2).
///
/// `flutter/lib/common/hbbs/hbbs.dart:180-197`: every key is optional, so
/// the 2FA first step (`{type: "email_check", tfa_type, secret}`) parses too
/// when it has no `user`; a challenge with its minimal `user` is checked with
/// [`SecondFactorChallenge`], what the login dialog actually needs.
/// Stricter than Flutter: `user` must be a full [`ClientUserPayload`], since
/// the same user record is what the Rust client reads elsewhere.
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

/// `POST /api/login` reply that asks for a second factor (03 §3.2), as the
/// login dialog uses it rather than as `LoginResponse.fromJson` merely parses
/// it (`flutter/lib/common/hbbs/hbbs.dart:180-197`, all optional there):
///
/// - `flutter/lib/common/widgets/login.dart:790-812`: `type` must be
///   `email_check`; `tfa_type` `tfa_check` opens the 2FA dialog (null or
///   `email_check` the email dialog, anything else is an error); `secret` and
///   `user` are handed to `verificationCodeDialog`.
/// - `login.dart:1003-1024`: the second request sends `secret` back and
///   `user?.name` as `username`, so without `user.name` the second step has no
///   username. `UserPayload.fromJson` (`hbbs.dart:36-47`) defaults every other
///   field, so a user object with only `name` is complete for the dialog.
#[derive(Debug, Deserialize)]
pub struct SecondFactorChallenge {
    #[serde(rename = "type")]
    pub kind: String,
    pub tfa_type: String,
    pub secret: String,
    pub user: ChallengeUser,
}

/// The `user` of a [`SecondFactorChallenge`]: what `verificationCodeDialog` reads.
#[derive(Debug, Deserialize)]
pub struct ChallengeUser {
    pub name: String,
}

/// `POST /api/oidc/auth` response (03 §3.5).
///
/// `src/hbbs_http/account.rs:24-28`: both required; `url` is a `url::Url`,
/// so it must be an absolute URL. Read through `HbbHttpResponse`
/// (`:160-177`), hence the shape is `HbbData<OidcAuthUrl>`.
#[derive(Debug, Deserialize)]
pub struct OidcAuthUrl {
    pub code: String,
    pub url: url::Url,
}

/// Completed OIDC login, read by the Rust client (03 §3.6).
///
/// `src/hbbs_http/account.rs:99-108`: `access_token`, `type` and `user`
/// required; `tfa_type` and `secret` default to `""` when missing, but a
/// `null` fails the whole body (the client then polls until it times out).
/// Read through `HbbHttpResponse` (`:179-198`), hence the shape is
/// `HbbData<AuthBody>`.
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

    /// The UserPayload example of 03 §3.3.
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
