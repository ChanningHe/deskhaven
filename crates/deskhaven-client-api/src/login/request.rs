//! The `/api/login` body (03 §3.2), as `LoginRequest.toJson` sends it
//! (`flutter/lib/common/hbbs/hbbs.dart:155-177`), validated at the boundary: nothing is
//! counted or verified for a malformed request.
//!
//! `Content-Type` must be `application/json` (parameters allowed, any case), as the client
//! sends it. Unknown fields are ignored (the client adds fields over time, 06 §6).

use axum::http::{HeaderMap, header};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::users::Password;
use serde::Deserialize;
use serde_json::Value;

use crate::ClientApiError;

/// `type` of a password login (`HttpType.kAuthReqTypeAccount`).
const ACCOUNT: &str = "account";
const JSON: &str = "application/json";
pub const INVALID_REQUEST_MESSAGE: &str = "Invalid login request";
pub const UNSUPPORTED_TYPE_MESSAGE: &str = "Unsupported login type";

/// No `Debug`: it holds the plaintext password until converted.
#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "type")]
    kind: Option<String>,
    username: Option<String>,
    password: Option<String>,
    id: Option<String>,
    uuid: Option<String>,
    #[serde(rename = "deviceInfo")]
    device_info: Option<Value>,
}

/// A login request by type.
#[derive(Debug)]
pub enum LoginRequest {
    /// `type: "account"` (or no type): username and password.
    Account(AccountLogin),
    /// Any other type. M2-08 adds `email_code` (the second step of a two-step login),
    /// later tasks their own types; until then they are refused.
    Unsupported,
}

#[derive(Debug)]
pub struct AccountLogin {
    pub username: String,
    pub password: Password,
    /// `id`, `uuid` and `deviceInfo`: what the session is bound to.
    pub device: ClientDevice,
}

impl LoginRequest {
    /// 400 [`INVALID_REQUEST_MESSAGE`] unless `body` is a well-formed login request.
    pub fn parse(body: &[u8]) -> Result<Self, ClientApiError> {
        let raw: Raw = serde_json::from_slice(body).map_err(|_| invalid())?;
        match raw.kind.as_deref() {
            None | Some(ACCOUNT) => raw.into_account().map(Self::Account),
            Some(_) => Ok(Self::Unsupported),
        }
    }
}

impl Raw {
    fn into_account(self) -> Result<AccountLogin, ClientApiError> {
        let (Some(username), Some(password), Some(id), Some(uuid)) =
            (self.username, self.password, self.id, self.uuid)
        else {
            return Err(invalid());
        };
        let device = ClientDevice {
            rustdesk_id: id,
            uuid,
            info: self.device_info.filter(|info| !info.is_null()),
        };
        if !is_rustdesk_id(&device.rustdesk_id) || !is_uuid(&device.uuid) {
            return Err(invalid());
        }
        device.check().map_err(|_| invalid())?;
        Ok(AccountLogin {
            username,
            password: Password::new(password),
            device,
        })
    }
}

/// 400 [`INVALID_REQUEST_MESSAGE`] unless the one `Content-Type` is `application/json`
/// (`; charset=...` and other parameters allowed, case-insensitive).
pub fn require_json(headers: &HeaderMap) -> Result<(), ClientApiError> {
    let mut values = headers.get_all(header::CONTENT_TYPE).iter();
    let is_json = match (values.next(), values.next()) {
        (Some(value), None) => value.to_str().is_ok_and(|v| {
            let essence = v.split(';').next().unwrap_or_default();
            essence.trim().eq_ignore_ascii_case(JSON)
        }),
        _ => false,
    };
    is_json.then_some(()).ok_or_else(invalid)
}

/// RustDesk ids are numeric or custom ids of ASCII letters, digits, `-` and `_`.
fn is_rustdesk_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The client's uuid is base64 of the machine id: printable ASCII, no spaces.
fn is_uuid(uuid: &str) -> bool {
    !uuid.is_empty() && uuid.bytes().all(|b| b.is_ascii_graphic())
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_REQUEST_MESSAGE.into())
}

#[cfg(test)]
mod tests {
    use deskhaven_core::sessions::{MAX_DEVICE_FIELD_LEN, MAX_DEVICE_INFO_LEN};
    use serde_json::json;

    use super::*;

    fn body(changes: Value) -> Vec<u8> {
        let mut value = json!({
            "username": "alice", "password": "pw", "id": "123456789",
            "uuid": "aGVsbG8+/w==", "autoLogin": true, "type": "account",
            "deviceInfo": { "os": "linux", "type": "client", "name": "host" },
            "futureField": 1
        });
        for (key, change) in changes.as_object().unwrap() {
            if change.is_null() {
                value.as_object_mut().unwrap().remove(key);
            } else {
                value[key] = change.clone();
            }
        }
        serde_json::to_vec(&value).unwrap()
    }

    fn account(changes: Value) -> Result<AccountLogin, ClientApiError> {
        match LoginRequest::parse(&body(changes))? {
            LoginRequest::Account(login) => Ok(login),
            LoginRequest::Unsupported => panic!("not an account login"),
        }
    }

    #[test]
    fn parses_what_the_client_sends() {
        let login = account(json!({})).unwrap();

        assert_eq!(login.username, "alice");
        assert_eq!(login.password.expose(), "pw");
        assert_eq!(login.device.rustdesk_id, "123456789");
        assert_eq!(login.device.uuid, "aGVsbG8+/w==");
        assert_eq!(
            login.device.info,
            Some(json!({"os": "linux", "type": "client", "name": "host"}))
        );
    }

    #[test]
    fn a_missing_type_is_an_account_login_and_others_are_unsupported() {
        assert!(account(json!({ "type": null })).is_ok());
        let other = LoginRequest::parse(&body(json!({ "type": "email_code" }))).unwrap();
        assert!(matches!(other, LoginRequest::Unsupported));
    }

    #[test]
    fn device_info_is_optional() {
        assert_eq!(
            account(json!({ "deviceInfo": null })).unwrap().device.info,
            None
        );
    }

    #[test]
    fn rejects_malformed_requests() {
        let long_id = "1".repeat(MAX_DEVICE_FIELD_LEN + 1);
        let huge_info = json!({ "name": "x".repeat(MAX_DEVICE_INFO_LEN) });
        for changes in [
            json!({ "username": null }),
            json!({ "password": null }),
            json!({ "id": null }),
            json!({ "uuid": null }),
            json!({ "id": "" }),
            json!({ "id": "12 34" }),
            json!({ "id": "1é" }),
            json!({ "id": long_id }),
            json!({ "id": 123_456_789 }),
            json!({ "uuid": "a b" }),
            json!({ "uuid": "a\u{0}" }),
            json!({ "deviceInfo": "linux" }),
            json!({ "deviceInfo": huge_info }),
            json!({ "password": 1 }),
        ] {
            assert!(account(changes.clone()).is_err(), "{changes}");
        }
        for raw in [&b""[..], b"{", b"null", b"\"x\""] {
            assert!(LoginRequest::parse(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn content_type_must_be_json_with_any_parameters_and_case() {
        let with = |values: &[&str]| {
            let mut headers = HeaderMap::new();
            for v in values {
                headers.append(header::CONTENT_TYPE, v.parse().unwrap());
            }
            require_json(&headers).is_ok()
        };
        for ok in [
            "application/json",
            "Application/JSON",
            "application/json; charset=utf-8",
            " application/json ;charset=UTF-8",
        ] {
            assert!(with(&[ok]), "{ok:?}");
        }
        for bad in [
            &[][..],
            &["text/plain; charset=utf-8"],
            &["application/x-www-form-urlencoded"],
            &["multipart/form-data; boundary=x"],
            &["application/jsonx"],
            &["application/json", "application/json"],
        ] {
            assert!(!with(bad), "{bad:?}");
        }
    }

    #[test]
    fn debug_never_shows_the_password() {
        let login = account(json!({ "password": "s3cret-value" })).unwrap();

        assert!(!format!("{login:?}").contains("s3cret-value"));
    }
}
