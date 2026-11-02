//! id + uuid verification of the device endpoints (06 §2, `devices.verify_uuid`).

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_db::entity::device;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use serde_json::json;

use super::support::{ID, b64, ctx, uuid_of};

#[tokio::test]
async fn a_registered_id_with_its_uuid_is_verified() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;

        // Act
        let verified = c.devices.verify(ID, &b64(&uuid_of(ID))).await.unwrap();

        // Assert
        assert_eq!(verified, Some(device_id), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn unknown_ids_wrong_uuids_and_malformed_base64_are_not_verified() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        c.device(ID).await;
        let unpadded = b64(&uuid_of(ID)).trim_end_matches('=').to_owned();

        // Act
        let other_uuid = b64(&uuid_of("555555555"));
        let unknown = c.devices.verify("555555555", &other_uuid).await.unwrap();
        let wrong = c.devices.verify(ID, &b64(b"another-uuid")).await.unwrap();
        let malformed = c.devices.verify(ID, "not base64!").await.unwrap();
        let empty = c.devices.verify(ID, "").await.unwrap();
        let no_padding = c.devices.verify(ID, &unpadded).await.unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(
            (unknown, wrong, malformed, empty),
            (None, None, None, None),
            "{b}"
        );
        assert!(
            no_padding.is_some(),
            "{b}: padding is optional, like hbbs keys"
        );
    })
    .await;
}

#[tokio::test]
async fn with_verify_uuid_off_any_uuid_passes_but_the_id_must_be_registered() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        c.set("devices.verify_uuid", json!(false)).await;

        // Act
        let wrong = c.devices.verify(ID, "anything").await.unwrap();
        let unknown = c.devices.verify("555555555", "anything").await.unwrap();

        // Assert
        assert_eq!((wrong, unknown), (Some(device_id), None), "{}", db.backend);
    })
    .await;
}

/// The identity of a verified device is served from memory; deleting the device through the
/// service forgets it.
#[tokio::test]
async fn a_verified_identity_is_cached_until_the_device_is_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let device_id = c.device(ID).await;
        let uuid = b64(&uuid_of(ID));
        c.devices.verify(ID, &uuid).await.unwrap();

        // Act: a row removed behind the service's back is still known from the cache ...
        device::Entity::delete_by_id(device_id)
            .exec(&c.conn)
            .await
            .unwrap();
        let cached = c.devices.verify(ID, &uuid).await.unwrap();
        // ... and a deletion through the service drops it.
        let again = c.device(ID).await;
        c.devices.delete(again).await.unwrap();
        let after_delete = c.devices.verify(ID, &uuid).await.unwrap();

        // Assert
        let b = db.backend;
        assert_eq!(cached, Some(device_id), "{b}");
        assert_eq!(after_delete, None, "{b}");
    })
    .await;
}
