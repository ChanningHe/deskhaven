//! `POST /api/devices/cli` (03 §2.9; `src/core_main.rs:530-626`): `rustdesk --assign --token
//! <t> [--user_name ...]`, with an API token holding the `assign` scope. The CLI prints
//! "Done!" for an empty body and any other body as it is.
//!
//! | case | status | body |
//! |---|---|---|
//! | placed | 200 | empty |
//! | no, unknown, revoked or expired token, or one without `assign` | 401 | `{"error": "Unauthorized"}` |
//! | uncached token from an address over its bearer budget | 429 + `Retry-After` | `{"error": "Please try later"}` |
//! | malformed body, missing `id` / `uuid`, nothing to assign, a malformed value | 400 | `{"error": ...}` naming the field |
//! | a name that matches nothing (checked only once the device is known) | 400 | `{"error": "<field> does not name an existing <kind>"}` |
//! | unknown id, or uuid mismatch | 404 | `{"error": "ID_NOT_FOUND"}` |
//! | body over [`MAX_ASSIGN_BODY`] | 413 | stack renderer |
//!
//! What each field does is `DeviceService::assign`'s.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::api_tokens::Scope;
use deskhaven_core::devices::{AddressBookPreset, DeviceAssignment, DeviceError};
use deskhaven_core::state::AppState;
use serde::Deserialize;

use super::{requester, token_auth};
use crate::ClientApiError;
use crate::client::Client;

/// Largest accepted body (bytes): every option at its longest is well under this.
pub const MAX_ASSIGN_BODY: usize = 16 * 1024;
/// Error text of an unknown id or a uuid mismatch (the heartbeat's word for it).
const ID_NOT_FOUND: &str = "ID_NOT_FOUND";
const INVALID_BODY: &str = "Invalid request body";

/// The body `core_main.rs:558-615` builds; unknown keys are ignored.
#[derive(Deserialize)]
struct Body {
    id: Option<String>,
    uuid: Option<String>,
    user_name: Option<String>,
    strategy_name: Option<String>,
    address_book_name: Option<String>,
    address_book_tag: Option<String>,
    address_book_alias: Option<String>,
    address_book_password: Option<String>,
    address_book_note: Option<String>,
    device_group_name: Option<String>,
    note: Option<String>,
    device_username: Option<String>,
    device_name: Option<String>,
}

pub async fn assign(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(), ClientApiError> {
    let principal = token_auth::authorize(&state, &headers, client, Scope::Assign).await?;
    let assignment = parse(&body)?;
    let by = requester(&principal, client);
    state.devices.assign(assignment, by).await.map_err(error)
}

fn parse(raw: &[u8]) -> Result<DeviceAssignment, ClientApiError> {
    let body: Body = serde_json::from_slice(raw).map_err(|_| invalid_body())?;
    let rustdesk_id = body.id.filter(|id| !id.is_empty());
    let uuid = body.uuid.filter(|uuid| !uuid.is_empty());
    let (Some(rustdesk_id), Some(uuid)) = (rustdesk_id, uuid) else {
        return Err(invalid_body());
    };
    let address_book = body.address_book_name.map(|name| AddressBookPreset {
        name,
        tag: body.address_book_tag,
        alias: body.address_book_alias,
        password: body.address_book_password.map(Into::into),
        note: body.address_book_note,
    });
    Ok(DeviceAssignment {
        rustdesk_id,
        uuid,
        user_name: body.user_name,
        strategy_name: body.strategy_name,
        device_group_name: body.device_group_name,
        note: body.note,
        device_username: body.device_username,
        device_name: body.device_name,
        address_book,
    })
}

fn error(err: DeviceError) -> ClientApiError {
    match err {
        DeviceError::UnknownDevice => ClientApiError::NotFound(ID_NOT_FOUND.into()),
        DeviceError::NameNotFound { .. } | DeviceError::Invalid { .. } => {
            ClientApiError::BadRequest(err.to_string().into())
        }
        other => ClientApiError::internal(other),
    }
}

fn invalid_body() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_BODY.into())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse_json(value: serde_json::Value) -> Result<DeviceAssignment, ClientApiError> {
        parse(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn parses_the_cli_body() {
        let a = parse_json(json!({
            "id": "123456789", "uuid": "dXVpZA==", "user_name": "pat",
            "address_book_name": "Team", "address_book_password": "p", "unknown": 1
        }))
        .unwrap();

        assert_eq!(
            (a.rustdesk_id.as_str(), a.uuid.as_str()),
            ("123456789", "dXVpZA==")
        );
        assert_eq!(a.user_name.as_deref(), Some("pat"));
        let book = a.address_book.unwrap();
        assert_eq!(book.name, "Team");
        assert_eq!(book.password.as_deref().map(String::as_str), Some("p"));
    }

    #[test]
    fn book_details_need_a_book_name() {
        let a = parse_json(json!({
            "id": "123456789", "uuid": "dXVpZA==", "address_book_tag": "t", "note": "n"
        }))
        .unwrap();

        assert!(a.address_book.is_none());
    }

    #[test]
    fn rejects_bodies_without_a_usable_id_or_uuid() {
        for body in [
            json!([]),
            json!({ "uuid": "dXVpZA==" }),
            json!({ "id": "123456789" }),
            json!({ "id": "", "uuid": "dXVpZA==" }),
            json!({ "id": "123456789", "uuid": "dXVpZA==", "note": 5 }),
        ] {
            assert!(
                matches!(parse_json(body.clone()), Err(ClientApiError::BadRequest(_))),
                "{body}"
            );
        }
    }
}
