//! `oidc_flows.context` (JSON, written and read only here): the device a client flow was
//! started from, and after a successful callback the resolved account's credential stamp.

use deskhaven_db::entity::user::AuthSource;
use sea_orm::ActiveEnum;
use serde_json::{Value, json};

use crate::sessions::{ClientDevice, TokenHash};
use crate::users::CredentialStamp;

pub(crate) fn for_client(device: &ClientDevice) -> Value {
    json!({
        "rustdesk_id": device.rustdesk_id.trim(),
        "uuid": device.uuid.trim(),
        "device_info": device.info.clone().unwrap_or(Value::Null),
    })
}

/// A console flow: the SHA-256 of the browser binding it must complete with.
pub(crate) fn for_console(binding: &TokenHash) -> Value {
    json!({ "binding": binding.as_str() })
}

/// The browser binding hash of a console flow; `None` when the context is not one.
pub(crate) fn console_binding(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    value.get("binding")?.as_str().map(str::to_owned)
}

/// The device of a client flow; `None` when the context is not one.
pub(crate) fn client_device(raw: &str) -> Option<ClientDevice> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let text = |key: &str| value.get(key)?.as_str().map(str::to_owned);
    Some(ClientDevice {
        rustdesk_id: text("rustdesk_id")?,
        uuid: text("uuid")?,
        info: value.get("device_info").filter(|v| v.is_object()).cloned(),
    })
}

/// `raw` with the stamp added; `None` when `raw` is not a JSON object.
pub(crate) fn with_stamp(raw: &str, stamp: CredentialStamp) -> Option<String> {
    let mut value: Value = serde_json::from_str(raw).ok()?;
    let (password_changed_at, source) = stamp.parts();
    value.as_object_mut()?.insert(
        "stamp".to_owned(),
        json!({
            "password_changed_at": password_changed_at,
            "auth_source": source.to_value(),
        }),
    );
    Some(value.to_string())
}

pub(crate) fn stamp(raw: &str) -> Option<CredentialStamp> {
    let value: Value = serde_json::from_str(raw).ok()?;
    let stamp = value.get("stamp")?;
    let password_changed_at = match stamp.get("password_changed_at")? {
        Value::Null => None,
        ms => Some(ms.as_i64()?),
    };
    let source = stamp.get("auth_source")?.as_str()?.to_owned();
    let source = AuthSource::try_from_value(&source).ok()?;
    Some(CredentialStamp::from_parts(password_changed_at, source))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn device_and_stamp_round_trip() {
        let device = ClientDevice {
            rustdesk_id: " 123456789 ".to_owned(),
            uuid: "dXVpZA==".to_owned(),
            info: Some(json!({"os": "linux"})),
        };
        let raw = for_client(&device).to_string();
        let stamp = CredentialStamp::from_parts(Some(42), AuthSource::Oidc);

        let stored = with_stamp(&raw, stamp).unwrap();
        let back = client_device(&stored).unwrap();

        assert_eq!(back.rustdesk_id, "123456789");
        assert_eq!(back.uuid, "dXVpZA==");
        assert_eq!(back.info, Some(json!({"os": "linux"})));
        assert_eq!(super::stamp(&stored), Some(stamp));
        assert_eq!(super::stamp(&raw), None);
    }

    #[test]
    fn malformed_contexts_read_as_absent() {
        assert!(client_device("[]").is_none());
        assert!(client_device(r#"{"rustdesk_id": 1, "uuid": "u"}"#).is_none());
        assert!(with_stamp("[]", CredentialStamp::from_stored(None)).is_none());
        assert!(stamp(r#"{"stamp": {"password_changed_at": null, "auth_source": "x"}}"#).is_none());
    }
}
