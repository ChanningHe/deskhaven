//! Snapshot redaction for console audit rows (see the parent module docs for the layers).
//!
//! An object member is masked when its key (ASCII case-insensitive):
//! - contains one of [`SECRET_KEY_FRAGMENTS`] (`password`, `passwd`, `smtp_pass` all
//!   contain `pass`),
//! - ends with one of [`SECRET_KEY_SUFFIXES`] (the schema's `*_enc` ciphertext and
//!   `*_hash` digest columns, `*_key`), or
//! - is one of [`SECRET_KEYS`].
//!
//! Independently of keys, any string **value** that looks like a credential is masked
//! wherever it appears: an API token (`dhk_` prefix, 06 §3.2) or a sealed secret
//! (`v1:<key_id>:<nonce>:<ciphertext>`, 04 §1 / 06 §5).

use deskhaven_config::REDACTED;
use serde_json::{Map, Value};

/// Keys containing any of these are secret.
pub const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "pass",
    "secret",
    "token",
    "private_key",
    "recovery_code",
    "credential",
    "seed",
];
/// Keys ending with any of these are secret.
pub const SECRET_KEY_SUFFIXES: &[&str] = &["_enc", "_hash", "_key"];
/// Keys equal to any of these are secret.
pub const SECRET_KEYS: &[&str] = &["hash", "nonce"];
/// Prefix of API tokens.
pub const API_TOKEN_PREFIX: &str = "dhk_";

const SEALED_VERSION: &str = "v1";
/// `v1`, key id, nonce, ciphertext.
const SEALED_PARTS: usize = 4;

/// `value` with every secret member (by key) and every credential-looking string replaced
/// by [`REDACTED`], at any depth. `null` stays `null`.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), redact_member(key, member)))
                .collect::<Map<_, _>>(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        Value::String(s) if is_credential_value(s) => masked(),
        scalar => scalar.clone(),
    }
}

/// Whether a member named `key` is masked by name.
pub fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS.iter().any(|f| key.contains(f))
        || SECRET_KEY_SUFFIXES.iter().any(|s| key.ends_with(s))
        || SECRET_KEYS.contains(&key.as_str())
}

fn redact_member(key: &str, member: &Value) -> Value {
    if member.is_null() || !is_secret_key(key) {
        return redact(member);
    }
    masked()
}

fn is_credential_value(s: &str) -> bool {
    s.starts_with(API_TOKEN_PREFIX) || is_sealed(s)
}

fn is_sealed(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() >= SEALED_PARTS
        && parts[0] == SEALED_VERSION
        && parts.iter().all(|part| !part.is_empty())
}

fn masked() -> Value {
    Value::String(REDACTED.to_owned())
}

#[cfg(test)]
mod tests {
    use deskhaven_config::Secret;
    use serde_json::json;

    use super::*;

    #[test]
    fn masks_secret_keys_at_any_depth_and_keeps_the_rest() {
        let input = json!({
            "username": "alice",
            "Password": "hunter2",
            "smtp": { "bind_passwd": 1, "smtp_pass": "x", "host": "mail" },
            "providers": [{ "client_secret": "s3", "name": "idp" }],
            "api_token": { "nested": "whole object" },
            "totp_secret": null,
        });

        let got = redact(&input);

        assert_eq!(
            got,
            json!({
                "username": "alice",
                "Password": REDACTED,
                "smtp": { "bind_passwd": REDACTED, "smtp_pass": REDACTED, "host": "mail" },
                "providers": [{ "client_secret": REDACTED, "name": "idp" }],
                "api_token": REDACTED,
                "totp_secret": null,
            })
        );
    }

    #[test]
    fn masks_suffixes_and_exact_keys_but_not_lookalikes() {
        let masked_keys = [
            "totp_secret_enc",
            "password_hash",
            "code_hash",
            "signing_key",
            "hash",
            "nonce",
            "Seed",
            "totp_seed",
        ];
        for key in masked_keys {
            assert!(is_secret_key(key), "{key}");
        }
        for key in [
            "key",
            "keys",
            "name",
            "hashes_count",
            "encoding",
            "keyboard",
            "id",
        ] {
            assert!(!is_secret_key(key), "{key}");
        }
    }

    #[test]
    fn masks_credential_values_under_any_key() {
        let input = json!({
            "note": "dhk_abcdef0123",
            "list": ["v1:k1:bm9uY2U:Y2lwaGVy", "plain", "v1:only:three"],
            "label": "v1::x:y",
        });

        let got = redact(&input);

        assert_eq!(
            got,
            json!({
                "note": REDACTED,
                "list": [REDACTED, "plain", "v1:only:three"],
                "label": "v1::x:y",
            })
        );
        assert_eq!(redact(&json!("dhk_x")), json!(REDACTED));
    }

    #[test]
    fn secret_wrapper_masks_innocuous_field_names() {
        #[derive(serde::Serialize)]
        struct SettingView {
            key: &'static str,
            value: Secret,
        }
        let view = SettingView {
            key: "smtp.password",
            value: Secret::new("hunter2"),
        };

        let got = redact(&serde_json::to_value(view).unwrap());

        assert_eq!(got, json!({ "key": "smtp.password", "value": REDACTED }));
    }

    #[test]
    fn ordinary_scalars_and_arrays_are_unchanged() {
        for value in [
            json!(1),
            json!("password"),
            json!([1, "token"]),
            json!(null),
        ] {
            assert_eq!(redact(&value), value);
        }
    }
}
