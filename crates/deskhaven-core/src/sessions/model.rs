//! Domain types of the session service.

use std::net::IpAddr;

use deskhaven_db::entity::session;
use deskhaven_db::entity::user::Role;
use deskhaven_db::time::TimestampMs;

pub use deskhaven_db::entity::session::Kind as SessionKind;

use super::SessionError;

/// Longest accepted `rustdesk_id` / `device_uuid` (characters).
pub const MAX_DEVICE_FIELD_LEN: usize = 128;
/// Largest accepted `device_info` / user agent, serialized (bytes).
pub const MAX_DEVICE_INFO_LEN: usize = 2048;

/// Who a valid token belongs to: what an authenticated request carries.
///
/// `role` is read at authentication time and may be up to the cache TTL old (role changes
/// emit no event, see [`UserEvent`](crate::users::UserEvent)), the bound RBAC accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPrincipal {
    pub session_id: i64,
    pub user_id: i64,
    pub role: Role,
    pub kind: SessionKind,
    /// Client sessions: the device the token was issued to.
    pub device: Option<DeviceBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceBinding {
    pub rustdesk_id: String,
    pub uuid: String,
}

/// The device a client session is issued to (`id`, `uuid`, `deviceInfo` of `/api/login`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClientDevice {
    pub rustdesk_id: String,
    pub uuid: String,
    /// `deviceInfo` as the client sent it (os, type, name); must be a JSON object.
    pub info: Option<serde_json::Value>,
}

impl ClientDevice {
    /// The checks [`SessionService::create_client`](super::SessionService::create_client)
    /// applies, for request validation before any work is done:
    /// [`SessionError::InvalidDevice`] for an empty, over-long or control-character id or
    /// uuid, or an info that is not a JSON object of at most [`MAX_DEVICE_INFO_LEN`] bytes.
    pub fn check(&self) -> Result<(), SessionError> {
        self.validate().map(drop)
    }

    /// Trimmed, non-empty, bounded fields and the serialized info.
    pub(super) fn validate(&self) -> Result<(DeviceBinding, Option<String>), SessionError> {
        let rustdesk_id = device_field("rustdesk_id", &self.rustdesk_id)?;
        let uuid = device_field("uuid", &self.uuid)?;
        let info = match &self.info {
            None => None,
            Some(value @ serde_json::Value::Object(_)) => Some(bounded_info(value.to_string())?),
            Some(_) => return Err(invalid("device_info", "must be a JSON object")),
        };
        Ok((DeviceBinding { rustdesk_id, uuid }, info))
    }
}

/// The stored `device_info` of a console session: `{"user_agent": ...}`, truncated.
pub(super) fn console_info(user_agent: Option<&str>) -> Option<String> {
    let agent = user_agent.map(str::trim).filter(|a| !a.is_empty())?;
    let cut = truncate(agent, MAX_DEVICE_INFO_LEN / 2);
    Some(serde_json::json!({ "user_agent": cut }).to_string())
}

/// A session as listed to its owner or an admin. Carries no token or token hash.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: i64,
    pub user_id: i64,
    pub kind: SessionKind,
    pub device: Option<DeviceBinding>,
    pub device_info: Option<serde_json::Value>,
    pub ip: Option<IpAddr>,
    pub created_at: TimestampMs,
    pub last_used_at: TimestampMs,
    pub expires_at: TimestampMs,
    pub revoked_at: Option<TimestampMs>,
}

impl From<session::Model> for Session {
    fn from(m: session::Model) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            kind: m.kind,
            device: binding(m.rustdesk_id, m.device_uuid),
            // Written by this service as JSON objects; anything else reads as absent.
            device_info: m.device_info.and_then(|s| serde_json::from_str(&s).ok()),
            ip: m.ip.and_then(|s| s.parse().ok()),
            created_at: m.created_at,
            last_used_at: m.last_used_at,
            expires_at: m.expires_at,
            revoked_at: m.revoked_at,
        }
    }
}

pub(super) fn binding(rustdesk_id: Option<String>, uuid: Option<String>) -> Option<DeviceBinding> {
    Some(DeviceBinding {
        rustdesk_id: rustdesk_id?,
        uuid: uuid?,
    })
}

fn device_field(field: &'static str, raw: &str) -> Result<String, SessionError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    if value.chars().count() > MAX_DEVICE_FIELD_LEN || value.chars().any(char::is_control) {
        return Err(invalid(field, "too long or contains control characters"));
    }
    Ok(value.to_owned())
}

fn bounded_info(serialized: String) -> Result<String, SessionError> {
    if serialized.len() > MAX_DEVICE_INFO_LEN {
        return Err(invalid("device_info", "too large"));
    }
    Ok(serialized)
}

fn truncate(s: &str, max_bytes: usize) -> &str {
    let end = (0..=max_bytes.min(s.len()))
        .rev()
        .find(|&i| s.is_char_boundary(i))
        .unwrap_or(0);
    &s[..end]
}

fn invalid(field: &'static str, reason: &'static str) -> SessionError {
    SessionError::InvalidDevice { field, reason }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn device(id: &str, uuid: &str, info: Option<serde_json::Value>) -> ClientDevice {
        ClientDevice {
            rustdesk_id: id.to_owned(),
            uuid: uuid.to_owned(),
            info,
        }
    }

    #[test]
    fn device_fields_are_trimmed_and_info_serialized() {
        let (binding, info) = device(" 123 ", "u-1", Some(json!({"os": "linux"})))
            .validate()
            .unwrap();

        assert_eq!(binding.rustdesk_id, "123");
        assert_eq!(binding.uuid, "u-1");
        assert_eq!(info.as_deref(), Some(r#"{"os":"linux"}"#));
    }

    #[test]
    fn device_rejects_empty_long_control_and_non_object_info() {
        let long = "x".repeat(MAX_DEVICE_FIELD_LEN + 1);
        let huge = json!({ "name": "x".repeat(MAX_DEVICE_INFO_LEN) });
        let cases = [
            (device("", "u", None), "rustdesk_id"),
            (device("  ", "u", None), "rustdesk_id"),
            (device("1", "", None), "uuid"),
            (device(&long, "u", None), "rustdesk_id"),
            (device("1\n2", "u", None), "rustdesk_id"),
            (device("1", "u", Some(json!("linux"))), "device_info"),
            (device("1", "u", Some(huge)), "device_info"),
        ];
        for (input, expected) in cases {
            match input.validate() {
                Err(SessionError::InvalidDevice { field, .. }) => assert_eq!(field, expected),
                other => panic!("{input:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn console_info_keeps_a_bounded_user_agent() {
        let long = "é".repeat(MAX_DEVICE_INFO_LEN);

        let stored = console_info(Some(&long)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&stored).unwrap();

        assert!(value["user_agent"].as_str().unwrap().len() <= MAX_DEVICE_INFO_LEN / 2);
        assert_eq!(console_info(Some("  ")), None);
        assert_eq!(console_info(None), None);
    }
}
