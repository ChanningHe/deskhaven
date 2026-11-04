//! Batch import of devices from the registry into a book as entries (rule 2).
//!
//! An entry takes what the registry knows: the RustDesk id, the administrator's name for the
//! device (alias), its hostname and user name, and the platform read off the reported OS. A
//! device the book already lists is left as it is. The whole import is one transaction: a
//! book without room for all the new entries takes none.

use std::collections::HashSet;

use deskhaven_db::entity::{address_book_peer, device};
use deskhaven_db::{begin_write, time};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::peer::PeerPatch;
use super::peers::peer_conflict;
use super::{AddressBookService, Rule, snapshot};
use crate::console_audit::TxGuard;

/// Most devices one import takes.
pub const MAX_IMPORT_DEVICES: usize = 500;
/// The longest text of an entry the client sends (`peer.rs`), so a registry value that
/// exceeds it is cut instead of refusing the whole import.
const SHORT_TEXT_MAX_CHARS: usize = 255;

/// What an import did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// RustDesk ids of the entries added.
    pub added: Vec<String>,
    /// RustDesk ids the book already listed (left as they are).
    pub existing: Vec<String>,
    /// `devices.id` values that name no device.
    pub missing: Vec<i64>,
}

impl AddressBookService {
    /// Adds the devices `device_ids` (`devices.id`) to book `guid` as entries; needs rule 2.
    pub async fn import_devices(
        &self,
        actor: Actor,
        guid: &str,
        device_ids: &[i64],
    ) -> Result<ImportReport, AddressBookError> {
        self.import_devices_guarded(actor, guid, device_ids, TxGuard::none())
            .await
    }

    /// [`import_devices`](Self::import_devices), with `guard` deciding on the ids added (no
    /// `before`; not consulted when nothing is added).
    pub async fn import_devices_guarded(
        &self,
        actor: Actor,
        guid: &str,
        device_ids: &[i64],
        guard: TxGuard<Value>,
    ) -> Result<ImportReport, AddressBookError> {
        if device_ids.len() > MAX_IMPORT_DEVICES {
            return Err(invalid(
                "device_ids",
                format!("at most {MAX_IMPORT_DEVICES} per request"),
            ));
        }
        let max = self.max_peers();
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::ReadWrite, true).await?;
        let wanted: HashSet<i64> = device_ids.iter().copied().collect();
        let mut devices = device::Entity::find()
            .filter(device::Column::Id.is_in(wanted.iter().copied()))
            .all(&tx)
            .await?;
        devices.sort_by(|a, b| a.rustdesk_id.cmp(&b.rustdesk_id));
        let found: HashSet<i64> = devices.iter().map(|d| d.id).collect();
        let mut report = ImportReport {
            missing: sorted(wanted.difference(&found).copied()),
            ..ImportReport::default()
        };
        let listed: HashSet<String> = address_book_peer::Entity::find()
            .filter(address_book_peer::Column::AddressBookId.eq(book.id))
            .all(&tx)
            .await?
            .into_iter()
            .map(|row| row.rustdesk_id)
            .collect();
        let (known, fresh): (Vec<_>, Vec<_>) = devices
            .into_iter()
            .partition(|d| listed.contains(&d.rustdesk_id));
        report.existing = known.into_iter().map(|d| d.rustdesk_id).collect();
        if (listed.len() + fresh.len()) as u64 > u64::from(max) {
            return Err(AddressBookError::Full { max });
        }
        let now = time::now();
        for device in &fresh {
            let patch = PeerPatch::from_json(entry_json(device))?;
            let row = self.new_row(&book, patch, now)?;
            row.insert(&tx).await.map_err(peer_conflict)?;
            report.added.push(device.rustdesk_id.clone());
        }
        if !fresh.is_empty() {
            touch(&tx, &book).await?;
            let after = snapshot::peer_ids(report.added.iter().map(String::as_str));
            guard.check(&tx, None, Some(&after)).await?;
        }
        tx.commit().await?;
        Ok(report)
    }
}

fn sorted(ids: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut ids: Vec<i64> = ids.collect();
    ids.sort_unstable();
    ids
}

/// The entry for `device`, as the client would send it.
fn entry_json(device: &device::Model) -> Value {
    let cut = |text: &str| text.chars().take(SHORT_TEXT_MAX_CHARS).collect::<String>();
    let username = device
        .display_username
        .as_deref()
        .or(device.username.as_deref());
    serde_json::json!({
        "id": device.rustdesk_id,
        "alias": cut(device.display_name.as_deref().unwrap_or_default()),
        "hostname": cut(device.hostname.as_deref().unwrap_or_default()),
        "username": cut(username.unwrap_or_default()),
        "platform": platform(device.os.as_deref().unwrap_or_default()),
    })
}

/// The platform name clients show an icon for, read off the reported OS text.
fn platform(os: &str) -> &'static str {
    let os = os.to_lowercase();
    [
        ("windows", "Windows"),
        ("linux", "Linux"),
        ("mac", "Mac OS"),
        ("android", "Android"),
    ]
    .into_iter()
    .find(|(needle, _)| os.contains(needle))
    .map_or("", |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_platform_is_read_off_the_os_text() {
        assert_eq!(platform("windows / Windows 10 Pro"), "Windows");
        assert_eq!(platform("linux / Ubuntu 22.04"), "Linux");
        assert_eq!(platform("macos / 14.2"), "Mac OS");
        assert_eq!(platform("android / 13"), "Android");
        assert_eq!(platform("plan9"), "");
        assert_eq!(platform(""), "");
    }
}
