//! [`DeviceService::assign`]: `rustdesk --assign` (03 §2.9, `src/core_main.rs:529-625`),
//! called with an API token holding the `assign` scope: an administrator's placement of a
//! device, by name.
//!
//! The device is the requester's own id + uuid, verified like on the other device endpoints
//! ([`DeviceError::UnknownDevice`] otherwise). Unlike the presets, an assignment **replaces**
//! what is set, and fails as a whole when a name matches nothing
//! ([`DeviceError::NameNotFound`]) or a value is malformed ([`DeviceError::Invalid`]):
//!
//! | field | effect |
//! |---|---|
//! | `user_name` | owner = the user with that username (case-insensitive) |
//! | `device_group_name` | device group = the group with exactly that name |
//! | `note` | device note |
//! | `device_name` / `device_username` | the hostname / username the device reports (until its sysinfo changes) |
//! | `strategy_name` | device-level strategy assignment = the strategy with that name |
//! | `address_book_*` | an entry in the shared book of that name, written as an administrator (see [`preset_book`](crate::devices::preset_book)) |
//!
//! Empty values count as absent; at least one field must be given. Every name and value is
//! checked before anything is written. Owner, device group, note and names are written in
//! one transaction; the strategy and the address book entry right after it commits, so a
//! failure there (a strategy deleted in between, a full book) leaves the row placed.

use std::sync::Arc;

use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, Entity as Devices};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, QuerySelect,
};

use super::{DeviceService, Inner};
use crate::addressbooks::{Actor, AddressBookError, AddressBookService};
use crate::devices::presets::{self, AddressBookPreset};
use crate::devices::sysinfo::MAX_FIELD_CHARS;
use crate::devices::{DeviceError, preset_book};
use crate::strategies::{StrategyError, Target};
use crate::text::{self, NOTE_MAX_LEN};

/// A `POST /api/devices/cli` body, as sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceAssignment {
    pub rustdesk_id: String,
    /// Base64 of the device uuid.
    pub uuid: String,
    pub user_name: Option<String>,
    pub strategy_name: Option<String>,
    pub device_group_name: Option<String>,
    pub note: Option<String>,
    pub device_username: Option<String>,
    pub device_name: Option<String>,
    /// `address_book_*`; only meaningful with a name.
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
        [
            &self.user_name,
            &self.strategy_name,
            &self.device_group_name,
            &self.note,
            &self.device_username,
            &self.device_name,
        ]
        .iter()
        .all(|v| v.is_none())
            && self.address_book.is_none()
    }
}

impl DeviceService {
    /// Places a device (see the module documentation).
    pub async fn assign(&self, assignment: DeviceAssignment) -> Result<(), DeviceError> {
        let a = assignment.without_blanks();
        validate(&a)?;
        let strategy_id = self.resolve_strategy(a.strategy_name.as_deref()).await?;
        let books = self.inner.address_books();
        let guid = resolve_book(&books, a.address_book.as_ref()).await?;
        let row_part = a.clone();
        let device = self.detached(move |inner| inner.place(row_part)).await?;
        if let Some(strategy_id) = strategy_id {
            let target = Target::Device(device.id);
            self.inner
                .strategies
                .assign(target, strategy_id)
                .await
                .map_err(strategy_error)?;
        }
        if let (Some(guid), Some(preset)) = (guid, &a.address_book) {
            preset_book::add_entry(&books, Actor::Admin, &guid, &device, preset)
                .await
                .map_err(book_error)?;
        }
        Ok(())
    }

    async fn resolve_strategy(&self, name: Option<&str>) -> Result<Option<i64>, DeviceError> {
        let Some(name) = name else {
            return Ok(None);
        };
        let strategy = self.inner.strategies.get_by_name(name).await;
        strategy.map(|s| Some(s.id)).map_err(strategy_error)
    }
}

impl Inner {
    fn address_books(&self) -> AddressBookService {
        AddressBookService::new(
            self.db.clone(),
            self.settings.clone(),
            Arc::clone(&self.crypto),
        )
    }

    /// The row half of [`DeviceService::assign`], under the row lock; returns the new row.
    async fn place(self: Arc<Self>, a: DeviceAssignment) -> Result<device::Model, DeviceError> {
        let tx = begin_write(&self.db).await?;
        let found = Devices::find()
            .filter(device::Column::RustdeskId.eq(&a.rustdesk_id))
            .lock_exclusive()
            .one(&tx)
            .await?;
        let row = found
            .filter(|row| self.uuid_matches(&row.uuid, &a.uuid))
            .ok_or(DeviceError::UnknownDevice)?;
        let owner = match &a.user_name {
            Some(name) => Some(
                presets::find_user(&tx, name)
                    .await?
                    .ok_or(not_found("user_name", "user"))?,
            ),
            None => None,
        };
        let group = match &a.device_group_name {
            Some(name) => Some(
                presets::find_device_group(&tx, name)
                    .await?
                    .ok_or(not_found("device_group_name", "device group"))?,
            ),
            None => None,
        };
        let moves = owner.is_some_and(|o| row.owner_user_id != Some(o))
            || group.is_some_and(|g| row.device_group_id != Some(g));
        let mut active = row.into_active_model();
        if let Some(owner) = owner {
            active.owner_user_id = Set(Some(owner));
        }
        if let Some(group) = group {
            active.device_group_id = Set(Some(group));
        }
        if let Some(note) = a.note {
            active.note = Set(Some(note));
        }
        if let Some(name) = a.device_name {
            active.hostname = Set(Some(name));
        }
        if let Some(name) = a.device_username {
            active.username = Set(Some(name));
        }
        active.updated_at = Set(time::now());
        let stored = active.update(&tx).await?;
        tx.commit().await?;
        if moves {
            self.strategies.cache().invalidate();
        }
        Ok(stored)
    }
}

/// The values themselves, before any name is looked up.
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

async fn resolve_book(
    books: &AddressBookService,
    preset: Option<&AddressBookPreset>,
) -> Result<Option<String>, DeviceError> {
    let Some(preset) = preset else {
        return Ok(None);
    };
    match books.find_shared_by_name(&preset.name).await {
        Ok(book) => Ok(Some(book.guid)),
        Err(AddressBookError::NotFound) => {
            Err(not_found("address_book_name", "shared address book"))
        }
        Err(AddressBookError::AmbiguousName) => Err(invalid(
            "address_book_name",
            "several shared address books have this name".to_owned(),
        )),
        Err(err) => Err(book_error(err)),
    }
}

fn given(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

fn invalid(field: &'static str, reason: String) -> DeviceError {
    DeviceError::Invalid { field, reason }
}

fn not_found(field: &'static str, kind: &'static str) -> DeviceError {
    DeviceError::NameNotFound { field, kind }
}

fn strategy_error(err: StrategyError) -> DeviceError {
    match err {
        StrategyError::NotFound => not_found("strategy_name", "strategy"),
        StrategyError::TargetNotFound(_) => DeviceError::UnknownDevice,
        other => DeviceError::Placement(Box::new(other)),
    }
}

/// Book failures the caller can fix are [`DeviceError::Invalid`]; the rest are internal.
fn book_error(err: AddressBookError) -> DeviceError {
    match err {
        AddressBookError::Invalid { .. } | AddressBookError::Full { .. } => {
            invalid("address_book", err.to_string())
        }
        other => DeviceError::Placement(Box::new(other)),
    }
}
