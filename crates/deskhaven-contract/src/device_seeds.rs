//! Device seeds (03 §2): a device the signal server registered, and what its sysinfo
//! presets can name. They write through the real registry and services of the app's state.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use deskhaven_core::addressbooks::{AddressBookService, NewSharedBook};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::devices::{DeviceRepository, Registration};
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::{NewStrategy, Options};
use deskhaven_core::users::{Credential, NewUser, Password};

use crate::seeds::{SEED_PASSWORD, SeedOutputs};

/// `registered-device`: the RustDesk id and the raw uuid it registered with.
pub const DEVICE_ID: &str = "135792468";
pub const DEVICE_UUID: &[u8] = b"contract-device-uuid";
/// [`DEVICE_UUID`] as the client sends it (`crate::encode64(get_uuid())`, padded base64).
pub const DEVICE_UUID_B64: &str = "Y29udHJhY3QtZGV2aWNlLXV1aWQ=";

/// `device-preset-targets`: what the presets of a sysinfo fixture name.
pub const PRESET_USER: &str = "pat";
pub const PRESET_DEVICE_GROUP: &str = "Office";
pub const PRESET_STRATEGY: &str = "Locked";
pub const PRESET_BOOK: &str = "Team";

/// A device registered through the signal server's store ([`DeviceRepository`]), with no
/// sysinfo yet. Outputs `device_id` (RustDesk id) and `device_uuid` (base64).
pub async fn registered_device(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let registration = Registration {
        rustdesk_id: DEVICE_ID.to_owned(),
        uuid: DEVICE_UUID.to_vec(),
        pk: vec![9; 32],
        ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)),
        at: SystemClock.now(),
    };
    DeviceRepository::new(state.db.clone())
        .upsert_registration(&registration)
        .await?;
    Ok(SeedOutputs::from([
        ("device_id".to_owned(), DEVICE_ID.to_owned()),
        ("device_uuid".to_owned(), DEVICE_UUID_B64.to_owned()),
    ]))
}

/// User [`PRESET_USER`], device group [`PRESET_DEVICE_GROUP`], strategy [`PRESET_STRATEGY`]
/// and admin-managed shared book [`PRESET_BOOK`]. No outputs.
pub async fn preset_targets(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let password = Password::new(SEED_PASSWORD);
    let user = NewUser::new(PRESET_USER, Credential::Local { password });
    state.users.create(user).await?;
    state.devices.create_group(PRESET_DEVICE_GROUP, "").await?;
    let options: Options = [("enable-file-transfer".to_owned(), "N".to_owned())]
        .into_iter()
        .collect();
    let strategy = NewStrategy::new(PRESET_STRATEGY, options);
    state.strategies.create(strategy).await?;
    let books = AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    );
    let book = NewSharedBook {
        name: PRESET_BOOK.to_owned(),
        note: String::new(),
        owner_user_id: None,
    };
    books.create_shared(book).await?;
    Ok(SeedOutputs::new())
}
