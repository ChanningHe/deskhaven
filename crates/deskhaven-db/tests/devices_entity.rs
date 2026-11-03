//! `devices` schema and entity against the migrated database, on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::sea_query::{InsertStatement, Query, SimpleExpr};
use sea_orm::{ActiveModelTrait, ActiveValue::NotSet, ActiveValue::Set, ConnectionTrait};

fn ts(ms: i64) -> TimestampMs {
    time::from_millis(ms).unwrap()
}

/// A row with only the required columns set.
fn device_row(rustdesk_id: &str) -> device::ActiveModel {
    let at = ts(1_700_000_000_123);
    device::ActiveModel {
        rustdesk_id: Set(rustdesk_id.to_owned()),
        uuid: Set(vec![0, 1, 2, 0xff]),
        pk: Set(vec![7; 32]),
        first_seen_at: Set(at),
        last_seen_at: Set(at),
        last_ip: Set("192.0.2.7".to_owned()),
        register_ip: Set("192.0.2.7".to_owned()),
        register_at: Set(at),
        created_at: Set(at),
        updated_at: Set(at),
        ..Default::default()
    }
}

#[tokio::test]
async fn device_round_trips_with_binary_keys_and_nullable_columns() {
    each_migrated_backend(async |db: &TestDb| {
        let owner = common::user_row("owner").insert(&db.conn).await.unwrap();
        let row = device::ActiveModel {
            id: NotSet,
            status: Set(DeviceStatus::Active),
            owner_user_id: Set(Some(owner.id)),
            hostname: Set(Some("host-1".to_owned())),
            ..device_row("123456789")
        };

        let stored = row.insert(&db.conn).await.unwrap();

        assert_eq!(stored.rustdesk_id, "123456789", "{}", db.backend);
        assert_eq!(stored.uuid, vec![0, 1, 2, 0xff], "{}", db.backend);
        assert_eq!(stored.pk, vec![7; 32], "{}", db.backend);
        assert_eq!(stored.owner_user_id, Some(owner.id), "{}", db.backend);
        assert_eq!(stored.device_group_id, None, "{}", db.backend);
        assert_eq!(stored.deployed_at, None, "{}", db.backend);
        assert_eq!(stored.register_at, ts(1_700_000_000_123), "{}", db.backend);
    })
    .await;
}

/// `display_name` / `display_username` (M2-19): the administrator's names for the device,
/// NULL until set, independent of the self-reported hostname / username.
#[tokio::test]
async fn display_names_are_nullable_and_separate_from_the_reported_ones() {
    each_migrated_backend(async |db: &TestDb| {
        let plain = device::ActiveModel {
            status: Set(DeviceStatus::Active),
            hostname: Set(Some("reported".to_owned())),
            ..device_row("111111111")
        };
        let named = device::ActiveModel {
            status: Set(DeviceStatus::Active),
            display_name: Set(Some("reception-01".to_owned())),
            display_username: Set(Some("frontdesk".to_owned())),
            ..device_row("222222222")
        };

        let plain = plain.insert(&db.conn).await.unwrap();
        let named = named.insert(&db.conn).await.unwrap();

        let b = db.backend;
        assert_eq!(
            (plain.display_name, plain.display_username),
            (None, None),
            "{b}"
        );
        assert_eq!(plain.hostname.as_deref(), Some("reported"), "{b}");
        assert_eq!(named.display_name.as_deref(), Some("reception-01"), "{b}");
        assert_eq!(named.display_username.as_deref(), Some("frontdesk"), "{b}");
        assert_eq!(named.hostname, None, "{b}");
    })
    .await;
}

#[tokio::test]
async fn rustdesk_id_is_unique() {
    each_migrated_backend(async |db: &TestDb| {
        let first = device::ActiveModel {
            status: Set(DeviceStatus::Active),
            ..device_row("123456789")
        };
        first.insert(&db.conn).await.unwrap();

        let second = device::ActiveModel {
            status: Set(DeviceStatus::Active),
            ..device_row("123456789")
        };
        let res = second.insert(&db.conn).await;

        assert!(
            res.is_err(),
            "{}: duplicate rustdesk_id accepted",
            db.backend
        );
    })
    .await;
}

/// The required columns of a raw insert, plus `extra` (bypasses the entity's status enum).
fn raw_insert(rustdesk_id: &str, extra: Option<(device::Column, &str)>) -> InsertStatement {
    use device::Column as C;
    let mut columns = vec![
        C::RustdeskId,
        C::Uuid,
        C::Pk,
        C::FirstSeenAt,
        C::LastSeenAt,
        C::LastIp,
        C::RegisterIp,
        C::RegisterAt,
        C::CreatedAt,
        C::UpdatedAt,
    ];
    let mut values: Vec<SimpleExpr> = vec![
        rustdesk_id.into(),
        vec![1u8].into(),
        vec![2u8].into(),
        1i64.into(),
        1i64.into(),
        "ip".into(),
        "ip".into(),
        1i64.into(),
        1i64.into(),
        1i64.into(),
    ];
    if let Some((col, value)) = extra {
        columns.push(col);
        values.push(value.into());
    }
    Query::insert()
        .into_table(device::Entity)
        .columns(columns)
        .values_panic(values)
        .to_owned()
}

/// The column defaults to `active` and its CHECK rejects anything else.
#[tokio::test]
async fn status_defaults_to_active_and_rejects_unknown_values() {
    each_migrated_backend(async |db: &TestDb| {
        db.conn
            .execute(&raw_insert("100000001", None))
            .await
            .unwrap();
        let bad = db
            .conn
            .execute(&raw_insert(
                "100000002",
                Some((device::Column::Status, "deleted")),
            ))
            .await;
        let row = device_by_id(db, "100000001").await;

        assert_eq!(row.status, DeviceStatus::Active, "{}", db.backend);
        assert!(bad.is_err(), "{}: unknown status accepted", db.backend);
    })
    .await;
}

async fn device_by_id(db: &TestDb, rustdesk_id: &str) -> device::Model {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    device::Entity::find()
        .filter(device::Column::RustdeskId.eq(rustdesk_id))
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}
