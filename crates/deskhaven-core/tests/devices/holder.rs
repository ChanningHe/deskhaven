//! Whom a RustDesk id belongs to (`DeviceService::id_holder`), for the OIDC client login: the
//! stored registration, else the signal server's memory.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::Ordering;

use deskhaven_core::devices::{DeviceRepository, IdHolder, Registration};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use serde_json::json;

use super::support::{ID, Live, T0_MS, b64, ctx, uuid_of};

const OTHER_ID: &str = "246813579";

#[tokio::test]
async fn a_stored_id_belongs_to_its_uuid_only() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let live = Live::new(Some(Vec::new()));
        c.devices.set_live_registrations(live.clone());

        // Act
        let own = c.devices.id_holder(ID, &b64(&uuid_of(ID))).await.unwrap();
        let other = c.devices.id_holder(ID, &b64(b"attacker")).await.unwrap();
        let malformed = c.devices.id_holder(ID, "not base64!").await.unwrap();
        let unknown = c
            .devices
            .id_holder(OTHER_ID, &b64(&uuid_of(OTHER_ID)))
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(own, IdHolder::ThisDevice, "{b}");
        assert_eq!(other, IdHolder::OtherDevice, "{b}");
        assert_eq!(malformed, IdHolder::OtherDevice, "{b}");
        assert_eq!(unknown, IdHolder::Nobody, "{b}");
        assert_eq!(
            live.asked.load(Ordering::SeqCst),
            1,
            "{b}: only the unstored id asks the signal server"
        );
    })
    .await;
}

/// A real client registers `get_uuid()` raw in RegisterPk and sends
/// `encode64(get_uuid())` (standard base64, padded) in its HTTP bodies.
#[tokio::test]
async fn a_real_clients_pair_is_the_same_machine_id_in_both_encodings() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let machine_id = b"6f1b2c3d4e5f60718293a4b5c6d7e8f9";
        let registration = Registration {
            rustdesk_id: ID.to_owned(),
            uuid: machine_id.to_vec(),
            pk: vec![9; 32],
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at: time::from_millis(T0_MS).unwrap(),
        };
        DeviceRepository::new(c.conn.clone())
            .upsert_registration(&registration)
            .await
            .unwrap();
        let sent = "NmYxYjJjM2Q0ZTVmNjA3MTgyOTNhNGI1YzZkN2U4Zjk=";

        // Act
        let holder = c.devices.id_holder(ID, sent).await.unwrap();

        // Assert
        assert_eq!(sent, b64(machine_id), "{}", db.backend);
        assert_eq!(holder, IdHolder::ThisDevice, "{}", db.backend);
    })
    .await;
}

/// Registrations are written behind: an id the signal server holds but has not stored yet
/// counts as registered; one it cannot answer for in time is unknown, not free.
#[tokio::test]
async fn an_unstored_id_is_asked_of_the_signal_server() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let known = vec![(ID.to_owned(), uuid_of(ID))];
        c.devices.set_live_registrations(Live::new(Some(known)));
        let unanswered = ctx(db).await;
        unanswered.devices.set_live_registrations(Live::new(None));

        // Act
        let own = c.devices.id_holder(ID, &b64(&uuid_of(ID))).await.unwrap();
        let other = c.devices.id_holder(ID, &b64(b"attacker")).await.unwrap();
        let free = c.devices.id_holder(OTHER_ID, &b64(b"any")).await.unwrap();
        let unknown = unanswered
            .devices
            .id_holder(ID, &b64(&uuid_of(ID)))
            .await
            .unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(own, IdHolder::ThisDevice, "{b}");
        assert_eq!(other, IdHolder::OtherDevice, "{b}");
        assert_eq!(free, IdHolder::Nobody, "{b}");
        assert_eq!(unknown, IdHolder::Unknown, "{b}");
    })
    .await;
}

/// `devices.verify_uuid` is about the device endpoints; the signal server never relaxes the
/// binding, so neither does this.
#[tokio::test]
async fn verify_uuid_off_does_not_hand_out_another_devices_id() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        c.set("devices.verify_uuid", json!(false)).await;

        // Act
        let other = c.devices.id_holder(ID, &b64(b"attacker")).await.unwrap();

        // Assert
        assert_eq!(other, IdHolder::OtherDevice, "{}", db.backend);
    })
    .await;
}
