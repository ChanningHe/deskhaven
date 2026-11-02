//! The `/api/login` body (03 §3.2), as `LoginRequest.toJson` sends it
//! (`flutter/lib/common/hbbs/hbbs.dart:155-177`), validated at the boundary: nothing is
//! counted or verified for a malformed request.
//!
//! `Content-Type` must be `application/json` (parameters allowed, any case), as the client
//! sends it. Unknown fields are ignored (the client adds fields over time, 06 §6).
//!
//! The second step of a two-step login (`verificationCodeDialog`,
//! `flutter/lib/common/widgets/login.dart:1003-1024`) has `type: "email_code"`, the
//! challenge `secret`, the `username` the first reply named (`user.name`), the device, and
//! the code in `verificationCode`, plus in `tfaCode` for a TOTP challenge (both carry the
//! same text). No password.

use std::fmt;

use axum::http::{HeaderMap, header};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::users::Password;
use serde::Deserialize;
use serde_json::Value;

use crate::ClientApiError;

/// `type` of a password login (`HttpType.kAuthReqTypeAccount`).
const ACCOUNT: &str = "account";
/// `type` of the second step (`HttpType.kAuthReqTypeEmailCode`), for TOTP too.
const EMAIL_CODE: &str = "email_code";
const JSON: &str = "application/json";
/// Longest accepted code (a TOTP code is 6 digits, a recovery code 11 characters).
pub const MAX_CODE_LEN: usize = 64;
/// Longest accepted challenge secret (ours are 43 characters).
pub const MAX_SECRET_LEN: usize = 128;
pub const INVALID_REQUEST_MESSAGE: &str = "Invalid login request";
pub const UNSUPPORTED_TYPE_MESSAGE: &str = "Unsupported login type";

/// No `Debug`: it holds the plaintext password or code until converted.
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
    secret: Option<String>,
    #[serde(rename = "verificationCode")]
    verification_code: Option<String>,
    #[serde(rename = "tfaCode")]
    tfa_code: Option<String>,
}

/// A login request by type.
#[derive(Debug)]
pub enum LoginRequest {
    /// `type: "account"` (or no type): username and password.
    Account(AccountLogin),
    /// `type: "email_code"`: the code answering a challenge of the first step.
    SecondStep(SecondStepLogin),
    /// Any other type (later tasks add their own); refused.
    Unsupported,
}

#[derive(Debug)]
pub struct AccountLogin {
    pub username: String,
    pub password: Password,
    /// `id`, `uuid` and `deviceInfo`: what the session is bound to.
    pub device: ClientDevice,
}

/// `Debug` shows neither the secret nor the code.
pub struct SecondStepLogin {
    pub username: String,
    pub secret: String,
    /// `tfaCode`, else `verificationCode`.
    pub code: String,
    pub device: ClientDevice,
}

impl fmt::Debug for SecondStepLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecondStepLogin")
            .field("username", &self.username)
            .field("device", &self.device)
            .finish_non_exhaustive()
    }
}

impl LoginRequest {
    /// 400 [`INVALID_REQUEST_MESSAGE`] unless `body` is a well-formed login request.
    pub fn parse(body: &[u8]) -> Result<Self, ClientApiError> {
        let raw: Raw = serde_json::from_slice(body).map_err(|_| invalid())?;
        match raw.kind.as_deref() {
            None | Some(ACCOUNT) => raw.into_account().map(Self::Account),
            Some(EMAIL_CODE) => raw.into_second_step().map(Self::SecondStep),
            Some(_) => Ok(Self::Unsupported),
        }
    }
}

impl Raw {
    fn into_account(self) -> Result<AccountLogin, ClientApiError> {
        let device = device(self.id, self.uuid, self.device_info)?;
        let (Some(username), Some(password)) = (self.username, self.password) else {
            return Err(invalid());
        };
        Ok(AccountLogin {
            username,
            password: Password::new(password),
            device,
        })
    }

    fn into_second_step(self) -> Result<SecondStepLogin, ClientApiError> {
        let device = device(self.id, self.uuid, self.device_info)?;
        let code = self.tfa_code.or(self.verification_code);
        let (Some(username), Some(secret), Some(code)) = (self.username, self.secret, code) else {
            return Err(invalid());
        };
        let bounded = |text: &str, max: usize| !text.is_empty() && text.len() <= max;
        if !bounded(&secret, MAX_SECRET_LEN) || !bounded(&code, MAX_CODE_LEN) {
            return Err(invalid());
        }
        Ok(SecondStepLogin {
            username,
            secret,
            code,
            device,
        })
    }
}

/// `id`, `uuid` and `deviceInfo`, checked as the session service will.
pub(crate) fn device(
    id: Option<String>,
    uuid: Option<String>,
    info: Option<Value>,
) -> Result<ClientDevice, ClientApiError> {
    let (Some(rustdesk_id), Some(uuid)) = (id, uuid) else {
        return Err(invalid());
    };
    let device = ClientDevice {
        rustdesk_id,
        uuid,
        info: info.filter(|info| !info.is_null()),
    };
    if !is_rustdesk_id(&device.rustdesk_id) || !is_uuid(&device.uuid) {
        return Err(invalid());
    }
    device.check().map_err(|_| invalid())?;
    Ok(device)
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
            other => panic!("not an account login: {other:?}"),
        }
    }

    /// The second step as `verificationCodeDialog` sends it for a TOTP challenge.
    fn second_step(changes: Value) -> Result<SecondStepLogin, ClientApiError> {
        let mut all = json!({
            "type": "email_code", "password": null, "secret": "s3cret-challenge",
            "verificationCode": "123456", "tfaCode": "123456"
        });
        for (key, change) in changes.as_object().unwrap() {
            all[key] = change.clone();
        }
        match LoginRequest::parse(&body(all))? {
            LoginRequest::SecondStep(step) => Ok(step),
            other => panic!("not a second step: {other:?}"),
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
        let mobile = LoginRequest::parse(&body(json!({ "type": "mobile" }))).unwrap();
        assert!(matches!(mobile, LoginRequest::Unsupported));
    }

    #[test]
    fn parses_the_second_step() {
        let step = second_step(json!({})).unwrap();

        assert_eq!(step.username, "alice");
        assert_eq!(step.secret, "s3cret-challenge");
        assert_eq!(step.code, "123456");
        assert_eq!(step.device.rustdesk_id, "123456789");
        // No tfaCode (an email challenge, M2-09): the verification code.
        let email = second_step(json!({ "tfaCode": null, "verificationCode": "654321" }));
        assert_eq!(email.unwrap().code, "654321");
    }

    #[test]
    fn rejects_malformed_second_steps() {
        let long = "1".repeat(MAX_CODE_LEN + 1);
        let long_secret = "a".repeat(MAX_SECRET_LEN + 1);
        for changes in [
            json!({ "secret": null }),
            json!({ "secret": "" }),
            json!({ "secret": long_secret }),
            json!({ "username": null }),
            json!({ "tfaCode": null, "verificationCode": null }),
            json!({ "tfaCode": "" }),
            json!({ "tfaCode": long }),
            json!({ "tfaCode": 123456 }),
            json!({ "id": null }),
            json!({ "uuid": "a b" }),
        ] {
            assert!(second_step(changes.clone()).is_err(), "{changes}");
        }
    }

    #[test]
    fn debug_never_shows_the_code_or_secret() {
        let step = second_step(json!({ "tfaCode": "987654" })).unwrap();

        let shown = format!("{step:?}");

        assert!(
            !shown.contains("987654") && !shown.contains("s3cret-challenge"),
            "{shown}"
        );
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
