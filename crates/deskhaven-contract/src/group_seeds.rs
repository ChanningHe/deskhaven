//! Group panel seeds (03 §4, 06 §4.2). Runs after `client-session` (user `alice`).

use std::net::{IpAddr, Ipv4Addr};

use anyhow::Context;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::devices::{DeviceRepository, Registration};
use deskhaven_core::groups::NewGroup;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Credential, NewUser, Password, Status};
use deskhaven_db::entity::device::{self, DeviceStatus};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use crate::seeds::{SEED_PASSWORD, SEED_USERNAME, SeedOutputs};

/// Shares the group `Team` with `alice`; active.
pub const TEAMMATE: &str = "tess";
/// Shares `Team` with `alice`; disabled.
pub const DISABLED_TEAMMATE: &str = "tom";
/// In the group `Elsewhere` only: nothing of hers is visible to `alice`.
pub const STRANGER: &str = "sam";

/// Devices of `group-scope`, by RustDesk id.
pub const ALICE_DEVICE: &str = "700000001";
pub const ALICE_DISABLED_DEVICE: &str = "700000006";
pub const TEAMMATE_DEVICE: &str = "700000002";
pub const DISABLED_TEAMMATE_DEVICE: &str = "700000003";
pub const STRANGER_DEVICE: &str = "700000004";
pub const UNOWNED_DEVICE: &str = "700000005";

/// What a seeded device looks like.
struct Spec {
    id: &'static str,
    owner: Option<i64>,
    hostname: &'static str,
    username: &'static str,
    os: &'static str,
    group: Option<i64>,
    note: &'static str,
    status: DeviceStatus,
}

async fn create_user(state: &AppState, username: &str, status: Status) -> anyhow::Result<i64> {
    let password = Password::new(SEED_PASSWORD);
    let new = NewUser {
        status,
        ..NewUser::new(username, Credential::Local { password })
    };
    Ok(state.users.create(new).await?.id)
}

async fn create_team(state: &AppState, name: &str, members: &[i64]) -> anyhow::Result<()> {
    let group = state.groups.create(NewGroup::new(name)).await?;
    for member in members {
        state.groups.add_member(group.id, *member).await?;
    }
    Ok(())
}

/// Registers the device, then gives it the sysinfo, owner, group and status of `spec`.
async fn add_device(state: &AppState, spec: Spec) -> anyhow::Result<()> {
    let registration = Registration {
        rustdesk_id: spec.id.to_owned(),
        uuid: format!("uuid-{}", spec.id).into_bytes(),
        pk: vec![7; 32],
        ip: IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9)),
        at: SystemClock.now(),
    };
    DeviceRepository::new(state.db.clone())
        .upsert_registration(&registration)
        .await?;
    let row = device::Entity::find()
        .filter(device::Column::RustdeskId.eq(spec.id))
        .one(&state.db)
        .await?
        .context("registered device is missing")?;
    let mut row: device::ActiveModel = row.into();
    row.hostname = Set(Some(spec.hostname.to_owned()));
    row.username = Set(Some(spec.username.to_owned()));
    row.os = Set(Some(spec.os.to_owned()));
    row.owner_user_id = Set(spec.owner);
    row.device_group_id = Set(spec.group);
    row.note = Set(Some(spec.note.to_owned()));
    row.status = Set(spec.status);
    row.update(&state.db).await?;
    Ok(())
}

/// Users [`TEAMMATE`], [`DISABLED_TEAMMATE`] and [`STRANGER`]; groups `Team` (alice, tess,
/// tom) and `Elsewhere` (sam); device groups `Office`, `Lab`, `Secret` and `Lobby`; and six
/// devices (ids above): alice's `Office` laptop, her disabled old PC, tess's `Lab` PC, tom's
/// ungrouped Mac, sam's `Secret` VM and an unowned one in `Lobby`. No outputs.
pub async fn group_scope(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let alice = state.users.get_by_username(SEED_USERNAME).await?.id;
    let tess = create_user(state, TEAMMATE, Status::Active).await?;
    let tom = create_user(state, DISABLED_TEAMMATE, Status::Disabled).await?;
    let sam = create_user(state, STRANGER, Status::Active).await?;
    create_team(state, "Team", &[alice, tess, tom]).await?;
    create_team(state, "Elsewhere", &[sam]).await?;
    let mut groups = Vec::new();
    for name in ["Office", "Lab", "Secret", "Lobby"] {
        groups.push(state.devices.create_group(name, "").await?);
    }
    let [office, lab, secret, lobby] = groups[..] else {
        anyhow::bail!("expected four device groups");
    };
    let spec = |id, owner, host, user, os, group, note| Spec {
        id,
        owner,
        hostname: host,
        username: user,
        os,
        group,
        note,
        status: DeviceStatus::Active,
    };
    let devices = [
        spec(
            ALICE_DEVICE,
            Some(alice),
            "alice-box",
            "alice",
            "linux / Ubuntu 24.04",
            Some(office),
            "mine",
        ),
        spec(
            TEAMMATE_DEVICE,
            Some(tess),
            "tess-pc",
            "tess",
            "windows / 11",
            Some(lab),
            "",
        ),
        spec(
            DISABLED_TEAMMATE_DEVICE,
            Some(tom),
            "tom-mac",
            "tom",
            "macos / 15",
            None,
            "",
        ),
        spec(
            STRANGER_DEVICE,
            Some(sam),
            "sam-vm",
            "sam",
            "linux / Debian",
            Some(secret),
            "",
        ),
        spec(
            UNOWNED_DEVICE,
            None,
            "lobby-pc",
            "kiosk",
            "windows / 10",
            Some(lobby),
            "",
        ),
        Spec {
            status: DeviceStatus::Disabled,
            ..spec(
                ALICE_DISABLED_DEVICE,
                Some(alice),
                "alice-old",
                "alice",
                "linux / Arch",
                None,
                "",
            )
        },
    ];
    for device in devices {
        add_device(state, device).await?;
    }
    Ok(SeedOutputs::new())
}
