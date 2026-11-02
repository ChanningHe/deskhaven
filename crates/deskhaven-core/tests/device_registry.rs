//! Device registry repository: registration upserts keyed by RustDesk id, uuid immutability.
//! Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, Ipv4Addr};

use deskhaven_core::devices::{DeviceError, DeviceRepository, Registration};
use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::{EntityTrait, PaginatorTrait};

const ID: &str = "123456789";

fn ts(ms: i64) -> TimestampMs {
    time::from_millis(ms).unwrap()
}

fn ip(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

fn registration(id: &str, uuid: &[u8], pk: &[u8], last: u8, at_ms: i64) -> Registration {
    Registration {
        rustdesk_id: id.to_owned(),
        uuid: uuid.to_vec(),
        pk: pk.to_vec(),
        ip: ip(last),
        at: ts(at_ms),
    }
}

async fn row(repo: &DeviceRepository, id: &str) -> device::Model {
    repo.find_by_rustdesk_id(id).await.unwrap().unwrap()
}

async fn count(db: &TestDb) -> u64 {
    device::Entity::find().count(&db.conn).await.unwrap()
}

#[tokio::test]
async fn first_registration_inserts_an_active_device() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let repo = DeviceRepository::new(db.conn.clone());

        // Act
        repo.upsert_registration(&registration(ID, b"uuid", b"pk1", 7, 1_000))
            .await
            .unwrap();

        // Assert
        let dev = row(&repo, ID).await;
        let b = db.backend;
        assert_eq!(
            (dev.uuid, dev.pk),
            (b"uuid".to_vec(), b"pk1".to_vec()),
            "{b}"
        );
        assert_eq!(dev.register_ip, "192.0.2.7", "{b}");
        assert_eq!(dev.register_at, ts(1_000), "{b}");
        assert_eq!(
            (dev.first_seen_at, dev.last_seen_at),
            (ts(1_000), ts(1_000)),
            "{b}"
        );
        assert_eq!(dev.last_ip, "192.0.2.7", "{b}");
        assert_eq!(dev.status, DeviceStatus::Active, "{b}");
        assert_eq!((dev.owner_user_id, dev.deployed_at), (None, None), "{b}");
    })
    .await;
}

#[tokio::test]
async fn find_unknown_id_is_none() {
    each_migrated_backend(async |db: &TestDb| {
        let repo = DeviceRepository::new(db.conn.clone());

        let found = repo.find_by_rustdesk_id(ID).await.unwrap();

        assert!(found.is_none(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn re_registration_updates_pk_ip_and_time_but_keeps_identity() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let repo = DeviceRepository::new(db.conn.clone());
        repo.upsert_registration(&registration(ID, b"uuid", b"pk1", 7, 1_000))
            .await
            .unwrap();
        let before = row(&repo, ID).await;

        // Act
        repo.upsert_registration(&registration(ID, b"uuid", b"pk2", 8, 2_000))
            .await
            .unwrap();

        // Assert
        let after = row(&repo, ID).await;
        let b = db.backend;
        assert_eq!(after.pk, b"pk2".to_vec(), "{b}");
        assert_eq!(after.register_ip, "192.0.2.8", "{b}");
        assert_eq!(after.register_at, ts(2_000), "{b}");
        assert!(*after.updated_at >= *before.updated_at, "{b}");
        assert_eq!(after.id, before.id, "{b}");
        assert_eq!(after.uuid, b"uuid".to_vec(), "{b}");
        assert_eq!(after.first_seen_at, before.first_seen_at, "{b}");
        assert_eq!(after.created_at, before.created_at, "{b}");
        assert_eq!(count(db).await, 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn other_uuid_is_rejected_and_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let repo = DeviceRepository::new(db.conn.clone());
        repo.upsert_registration(&registration(ID, b"uuid", b"pk1", 7, 1_000))
            .await
            .unwrap();
        let before = row(&repo, ID).await;

        // Act
        let res = repo
            .upsert_registration(&registration(ID, b"thief", b"pk2", 9, 2_000))
            .await;

        // Assert
        let b = db.backend;
        match res {
            Err(DeviceError::UuidMismatch { rustdesk_id }) => assert_eq!(rustdesk_id, ID),
            other => panic!("{b}: expected UuidMismatch, got {other:?}"),
        }
        assert_eq!(row(&repo, ID).await, before, "{b}");
    })
    .await;
}

#[tokio::test]
async fn ids_are_separate_rows() {
    each_migrated_backend(async |db: &TestDb| {
        let repo = DeviceRepository::new(db.conn.clone());

        repo.upsert_registration(&registration("111111111", b"a", b"pk-a", 1, 1_000))
            .await
            .unwrap();
        repo.upsert_registration(&registration("222222222", b"b", b"pk-b", 2, 1_000))
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(count(db).await, 2, "{b}");
        assert_eq!(row(&repo, "111111111").await.pk, b"pk-a".to_vec(), "{b}");
        assert_eq!(row(&repo, "222222222").await.pk, b"pk-b".to_vec(), "{b}");
    })
    .await;
}

/// Two first registrations of one id racing: the insert conflicts on the unique
/// `rustdesk_id` instead of failing, and the loser updates the winner's row.
#[tokio::test]
async fn concurrent_first_registrations_leave_one_row() {
    each_migrated_backend(async |db: &TestDb| {
        let repo = DeviceRepository::new(db.conn.clone());
        let first = registration(ID, b"uuid", b"pk1", 7, 1_000);
        let second = registration(ID, b"uuid", b"pk2", 7, 1_001);

        let (a, b) = tokio::join!(
            repo.upsert_registration(&first),
            repo.upsert_registration(&second)
        );

        let backend = db.backend;
        assert!(a.is_ok() && b.is_ok(), "{backend}: {a:?} {b:?}");
        assert_eq!(count(db).await, 1, "{backend}");
    })
    .await;
}
