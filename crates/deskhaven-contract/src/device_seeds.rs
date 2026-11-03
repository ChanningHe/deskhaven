//! Device seeds (03 §2): a device the signal server registered, and what its sysinfo
//! presets can name. They write through the real registry and services of the app's state.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use anyhow::Context;
use deskhaven_config::settings::{Change, DEVICES_ACCEPT_PRESETS};
use deskhaven_core::addressbooks::{Actor, AddressBookService, NewSharedBook, Rule, Subject};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::devices::{
    DeviceRepository, Presets, Registration, Sysinfo, SysinfoOutcome, SysinfoReport,
};
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::{NewStrategy, Options, Target};
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
/// and admin-managed shared book [`PRESET_BOOK`], on which the user holds rule 2. No outputs.
pub async fn preset_targets(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let password = Password::new(SEED_PASSWORD);
    let user = NewUser::new(PRESET_USER, Credential::Local { password });
    let user = state.users.create(user).await?;
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
    let book = books.create_shared(book).await?;
    let subject = Subject::User(user.id);
    books
        .set_share(Actor::Admin, &book.guid, subject, Rule::ReadWrite)
        .await?;
    Ok(SeedOutputs::new())
}

/// Sets `devices.accept_presets` to `all` (off by default). No outputs.
pub async fn presets_accepted(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let key = DEVICES_ACCEPT_PRESETS.key();
    let version = state
        .settings
        .snapshot()
        .view(key)
        .with_context(|| format!("unknown setting {key:?}"))?
        .version;
    let change = Change {
        key: key.to_owned(),
        value: serde_json::json!("all"),
        version,
    };
    let request = UpdateRequest {
        changes: vec![change],
        actor: None,
        reason: None,
    };
    state.settings.update(request).await?;
    Ok(SeedOutputs::new())
}

/// `global-strategy`: what the global strategy [`GLOBAL_STRATEGY`] carries; the empty value
/// tells the client to delete that option.
pub const GLOBAL_STRATEGY: &str = "Defaults";
pub const GLOBAL_OPTIONS: &[(&str, &str)] = &[
    ("enable-file-transfer", "N"),
    ("allow-remote-config-modification", ""),
];
/// `device-disconnect`: the connections queued for closing on [`DEVICE_ID`].
pub const QUEUED_DISCONNECTS: &[i32] = &[12, 13];

/// `device-sysinfo`: stores a sysinfo of [`DEVICE_ID`] (after `registered-device`), so its
/// heartbeat no longer asks for one. No outputs.
pub async fn device_sysinfo(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let report = SysinfoReport {
        rustdesk_id: DEVICE_ID.to_owned(),
        uuid: DEVICE_UUID_B64.to_owned(),
        sysinfo: Sysinfo {
            hostname: Some("office-pc".to_owned()),
            os: Some("linux / Ubuntu 24.04".to_owned()),
            version: Some("1.4.1".to_owned()),
            ..Sysinfo::default()
        },
        presets: Presets::default(),
    };
    let outcome = state.devices.report_sysinfo(report).await?;
    anyhow::ensure!(
        outcome == SysinfoOutcome::Stored,
        "sysinfo not stored: {outcome:?}"
    );
    Ok(SeedOutputs::new())
}

/// `global-strategy`: strategy [`GLOBAL_STRATEGY`] with [`GLOBAL_OPTIONS`], assigned to
/// every device. No outputs (it moves the strategy version: seed `strategy-version` after it).
pub async fn global_strategy(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let options: Options = GLOBAL_OPTIONS
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let strategy = NewStrategy::new(GLOBAL_STRATEGY, options);
    let strategy = state.strategies.create(strategy).await?;
    state.strategies.assign(Target::Global, strategy.id).await?;
    Ok(SeedOutputs::new())
}

/// `strategy-version`: outputs `strategy_modified_at`, the strategy version the client of an
/// up-to-date device holds (send it as `"${int:strategy_modified_at}"`).
pub async fn strategy_version(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let version = state.strategies.cache().version();
    Ok(SeedOutputs::from([(
        "strategy_modified_at".to_owned(),
        version.to_string(),
    )]))
}

/// `device-disconnect`: queues [`QUEUED_DISCONNECTS`] for closing on [`DEVICE_ID`]. No
/// outputs.
pub async fn device_disconnect(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let queue = state.devices.disconnects();
    queue.disconnect(DEVICE_ID, QUEUED_DISCONNECTS)?;
    Ok(SeedOutputs::new())
}
