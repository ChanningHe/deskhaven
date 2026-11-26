//! An assignment replaces what is set, all or nothing, in one transaction run in its own task.
//! The device is resolved before any name, so an unknown device reveals nothing about which
//! names exist. The uuid is always checked (constant time), whatever `devices.verify_uuid` says:
//! tokens travel in scripts and prove nothing about the machine asking.

use deskhaven_db::entity::device;

use super::{DeviceService, Requester};
use crate::addressbooks::AddressBookError;
use crate::devices::presets::AddressBookPreset;
use crate::devices::sysinfo::MAX_FIELD_CHARS;
use crate::devices::{DeviceError, preset_book};
use crate::strategies::StrategyError;
use crate::text::{self, NOTE_MAX_LEN};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceAssignment {
    pub rustdesk_id: String,
    pub uuid: String,
    pub user_name: Option<String>,
    pub strategy_name: Option<String>,
    pub device_group_name: Option<String>,
    pub note: Option<String>,
    pub device_username: Option<String>,
    pub device_name: Option<String>,
    pub address_book: Option<AddressBookPreset>,
}

impl DeviceAssignment {
    fn without_blanks(self) -> Self {
        let address_book = self
            .address_book
            .filter(|book| !book.name.is_empty())
            .map(|book| AddressBookPreset {
                tag: given(book.tag),
                alias: given(book.alias),
                note: given(book.note),
                password: book.password.filter(|p| !p.is_empty()),
                ..book
            });
        Self {
            user_name: given(self.user_name),
            strategy_name: given(self.strategy_name),
            device_group_name: given(self.device_group_name),
            note: given(self.note),
            device_username: given(self.device_username),
            device_name: given(self.device_name),
            address_book,
            ..self
        }
    }

    fn is_empty(&self) -> bool {
        let fields = [
            &self.user_name,
            &self.strategy_name,
            &self.device_group_name,
            &self.note,
            &self.device_username,
            &self.device_name,
        ];
        fields.iter().all(|v| v.is_none()) && self.address_book.is_none()
    }
}

impl DeviceService {
    pub async fn assign(
        &self,
        assignment: DeviceAssignment,
        by: Requester,
    ) -> Result<(), DeviceError> {
        let a = assignment.without_blanks();
        validate(&a)?;
        self.detached(move |inner| inner.assign_all(a, by)).await
    }
}

fn validate(a: &DeviceAssignment) -> Result<(), DeviceError> {
    if a.is_empty() {
        return Err(invalid("body", "names nothing to assign".to_owned()));
    }
    let texts = [
        ("note", a.note.as_deref(), NOTE_MAX_LEN, true),
        (
            "device_name",
            a.device_name.as_deref(),
            MAX_FIELD_CHARS,
            false,
        ),
        (
            "device_username",
            a.device_username.as_deref(),
            MAX_FIELD_CHARS,
            false,
        ),
    ];
    for (field, value, max, multiline) in texts {
        if let Some(reason) = value.and_then(|v| text::problem(v, max, multiline)) {
            return Err(invalid(field, reason));
        }
    }
    match &a.address_book {
        Some(book) => preset_book::check(&a.rustdesk_id, book).map_err(book_error),
        None => Ok(()),
    }
}

/// Never the password itself.
pub(super) fn book_view(a: &DeviceAssignment) -> serde_json::Value {
    let Some(b) = &a.address_book else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "name": b.name, "tag": b.tag, "alias": b.alias, "note": b.note,
        "has_password": b.password.is_some().then_some(true),
    })
}

pub(super) fn placement(row: &device::Model, strategy_id: Option<i64>) -> serde_json::Value {
    serde_json::json!({
        "owner_user_id": row.owner_user_id, "device_group_id": row.device_group_id,
        "note": row.note, "display_name": row.display_name,
        "display_username": row.display_username, "strategy_id": strategy_id,
    })
}

fn given(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

pub(super) fn invalid(field: &'static str, reason: String) -> DeviceError {
    DeviceError::Invalid { field, reason }
}

pub(super) fn not_found(field: &'static str, kind: &'static str) -> DeviceError {
    DeviceError::NameNotFound { field, kind }
}

pub(super) fn strategy_error(err: StrategyError) -> DeviceError {
    match err {
        StrategyError::NotFound => not_found("strategy_name", "strategy"),
        StrategyError::TargetNotFound(_) => DeviceError::UnknownDevice,
        other => DeviceError::Strategy(other),
    }
}

pub(super) fn book_error(err: AddressBookError) -> DeviceError {
    match err {
        AddressBookError::Invalid { .. } | AddressBookError::Full { .. } => {
            invalid("address_book", err.to_string())
        }
        other => DeviceError::AddressBook(other),
    }
}
