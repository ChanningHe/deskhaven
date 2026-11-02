//! UserPayload (03-client-api-contract §3.3): `currentUser`, the `user` of a login reply,
//! and later the items of `/api/users`. Read leniently by Flutter
//! (`flutter/lib/common/hbbs/hbbs.dart:36-48`) and strictly by the Rust client
//! (`src/hbbs_http/account.rs:53-97`: `name` and `info` required, `status` only 0/1/-1).

use std::collections::BTreeMap;

use deskhaven_core::users::{Role, Status, User};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct UserPayload {
    pub name: String,
    pub display_name: String,
    /// Server-relative (`/…`): the client prefixes its API address (03 §3.3).
    pub avatar: Option<String>,
    pub email: Option<String>,
    pub note: String,
    /// 1 normal, 0 disabled, -1 unverified.
    pub status: i8,
    /// Stored by the client, not used for anything.
    pub is_admin: bool,
    pub info: UserInfo,
    pub third_auth_type: Option<String>,
    pub verifier: Option<String>,
}

/// `user.info`. Must be present (the Rust client fails the whole payload otherwise).
#[derive(Debug, Serialize)]
pub struct UserInfo {
    /// Email code on login (M2-09); off until then.
    pub email_verification: bool,
    pub email_alarm_notification: bool,
    /// Pro's device whitelist; not offered.
    pub login_device_whitelist: Vec<serde_json::Value>,
    pub other: BTreeMap<String, String>,
}

impl From<&User> for UserPayload {
    fn from(user: &User) -> Self {
        Self {
            name: user.username.clone(),
            display_name: user.display_name.clone(),
            avatar: user.avatar_path.as_deref().and_then(avatar_url),
            email: user.email.clone(),
            note: user.note.clone(),
            status: client_status(user.status),
            is_admin: matches!(user.role, Role::Owner | Role::Admin),
            info: UserInfo {
                email_verification: false,
                email_alarm_notification: false,
                login_device_whitelist: Vec::new(),
                other: BTreeMap::new(),
            },
            third_auth_type: None,
            verifier: None,
        }
    }
}

const fn client_status(status: Status) -> i8 {
    match status {
        Status::Active => 1,
        Status::Disabled => 0,
        Status::Unverified => -1,
    }
}

/// The stored avatar path as a server-relative URL path; blank is no avatar.
fn avatar_url(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }
    Some(if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    })
}

#[cfg(test)]
mod tests {
    use deskhaven_core::users::AuthSource;
    use serde_json::json;

    use super::*;

    fn user(status: Status, role: Role, avatar: Option<&str>) -> User {
        let t = deskhaven_db::time::from_millis(0).unwrap();
        User {
            id: 7,
            username: "alice".into(),
            display_name: "Alice".into(),
            email: Some("a@x".into()),
            email_verified: true,
            auth_source: AuthSource::Local,
            role,
            status,
            must_change_password: false,
            totp_enabled: false,
            note: "n".into(),
            avatar_path: avatar.map(str::to_owned),
            last_login_at: None,
            created_at: t,
            updated_at: t,
        }
    }

    #[test]
    fn maps_like_the_contract_example() {
        let payload = UserPayload::from(&user(Status::Active, Role::User, Some("avatars/1.png")));

        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            json!({
                "name": "alice", "display_name": "Alice", "avatar": "/avatars/1.png",
                "email": "a@x", "note": "n", "status": 1, "is_admin": false,
                "info": { "email_verification": false, "email_alarm_notification": false,
                          "login_device_whitelist": [], "other": {} },
                "third_auth_type": null, "verifier": null
            })
        );
    }

    #[test]
    fn statuses_roles_and_avatars() {
        let status = |s| UserPayload::from(&user(s, Role::User, None)).status;
        assert_eq!(
            [
                status(Status::Active),
                status(Status::Disabled),
                status(Status::Unverified)
            ],
            [1, 0, -1]
        );
        let admin = |r| UserPayload::from(&user(Status::Active, r, None)).is_admin;
        assert_eq!(
            [admin(Role::Owner), admin(Role::Admin), admin(Role::User)],
            [true, true, false]
        );
        assert_eq!(avatar_url("/a/b.png").as_deref(), Some("/a/b.png"));
        assert_eq!(avatar_url("  "), None);
    }
}
