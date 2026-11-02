//! Row builders and assertions for the device, address book, strategy and audit schema tests
//! (M2-02).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::panic)]

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::{
    address_book, address_book_peer, address_book_share, address_book_tag, alarm,
    connection_session, console_audit, device, device_group, file_transfer, login_event, strategy,
    strategy_assignment,
};
use deskhaven_db::testing::TestDb;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{DatabaseConnection, DbErr, SqlErr};
use sea_orm_migration::prelude::SchemaManager;

use super::{T0_MS, exec, ts};

pub fn is_unique_violation(res: Result<impl Sized, DbErr>) -> bool {
    matches!(
        res.err().and_then(|e| e.sql_err()),
        Some(SqlErr::UniqueConstraintViolation(_))
    )
}

pub fn is_fk_violation(res: Result<impl Sized, DbErr>) -> bool {
    res.err()
        .is_some_and(|err| deskhaven_db::is_foreign_key_violation(&err))
}

pub async fn has_index(conn: &DatabaseConnection, table: &str, index: &str) -> bool {
    SchemaManager::new(conn)
        .has_index(table, index)
        .await
        .unwrap()
}

/// Asserts that every `(table, index)` exists on the backend under test.
pub async fn assert_indexes(db: &TestDb, expected: &[(&str, &str)]) {
    for (table, index) in expected {
        assert!(
            has_index(&db.conn, table, index).await,
            "{}: missing {index} on {table}",
            db.backend
        );
    }
}

/// `(table, bad assignment, good assignment)`, run as `UPDATE <table> SET ...` on a seeded row.
/// The good assignment is a control: the same statement shape succeeds with a valid value.
pub type CheckCase = (&'static str, &'static str, &'static str);

/// Runs `cases` in order on the seeded rows: each bad assignment must be rejected, then its
/// good control accepted (so a good assignment may change the row for later cases).
pub async fn assert_checks(db: &TestDb, cases: &[CheckCase]) {
    for (table, bad, good) in cases {
        let bad_sql = format!("UPDATE {table} SET {bad}");
        let good_sql = format!("UPDATE {table} SET {good}");
        let Err(err) = exec(&db.conn, &bad_sql).await else {
            panic!("{}: accepted: {bad_sql}", db.backend);
        };
        // A constraint of the kind under test, not a uniqueness or reference failure.
        assert!(
            !matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
                && !deskhaven_db::is_foreign_key_violation(&err),
            "{}: wrong kind of rejection: {bad_sql}: {err}",
            db.backend
        );
        exec(&db.conn, &good_sql)
            .await
            .unwrap_or_else(|e| panic!("{}: control rejected: {good_sql}: {e}", db.backend));
    }
}

pub fn device_row(rustdesk_id: &str) -> device::ActiveModel {
    device::ActiveModel {
        rustdesk_id: Set(rustdesk_id.to_owned()),
        uuid: Set(vec![0, 1, 2, 0xff]),
        pk: Set(vec![7; 32]),
        first_seen_at: Set(ts(T0_MS)),
        last_seen_at: Set(ts(T0_MS)),
        last_ip: Set("192.0.2.7".to_owned()),
        register_ip: Set("192.0.2.7".to_owned()),
        register_at: Set(ts(T0_MS)),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
        ..Default::default()
    }
}

pub fn device_group_row(name: &str) -> device_group::ActiveModel {
    device_group::ActiveModel {
        id: NotSet,
        name: Set(name.to_owned()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn address_book_row(
    owner_user_id: i64,
    kind: address_book::Kind,
    guid: &str,
) -> address_book::ActiveModel {
    address_book::ActiveModel {
        id: NotSet,
        guid: Set(guid.to_owned()),
        kind: Set(kind),
        owner_user_id: Set(Some(owner_user_id)),
        name: Set(guid.to_owned()),
        note: Set(String::new()),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn share_row(
    address_book_id: i64,
    subject_type: address_book_share::SubjectType,
    subject_id: i64,
    rule: address_book_share::Rule,
) -> address_book_share::ActiveModel {
    address_book_share::ActiveModel {
        address_book_id: Set(address_book_id),
        subject_type: Set(subject_type),
        subject_id: Set(subject_id),
        rule: Set(rule),
    }
}

pub fn peer_row(address_book_id: i64, rustdesk_id: &str) -> address_book_peer::ActiveModel {
    address_book_peer::ActiveModel {
        address_book_id: Set(address_book_id),
        rustdesk_id: Set(rustdesk_id.to_owned()),
        alias: Set("desk".to_owned()),
        note: Set(String::new()),
        tags: Set(r#"["office"]"#.to_owned()),
        hash: Set(None),
        password_enc: Set(None),
        username: Set("root".to_owned()),
        hostname: Set("host".to_owned()),
        platform: Set("Linux".to_owned()),
        force_always_relay: Set(false),
        rdp_port: Set(String::new()),
        rdp_username: Set(String::new()),
        login_name: Set(String::new()),
        extra: Set("{}".to_owned()),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn tag_row(address_book_id: i64, name: &str) -> address_book_tag::ActiveModel {
    address_book_tag::ActiveModel {
        address_book_id: Set(address_book_id),
        name: Set(name.to_owned()),
        color: Set(0xff12_34ab),
    }
}

pub fn strategy_row(name: &str) -> strategy::ActiveModel {
    strategy::ActiveModel {
        id: NotSet,
        name: Set(name.to_owned()),
        options: Set(r#"{"enable-audio":"N"}"#.to_owned()),
        note: Set(String::new()),
        managed_by: Set(ManagedBy::Ui),
        created_at: Set(ts(T0_MS)),
        updated_at: Set(ts(T0_MS)),
    }
}

pub fn assignment_row(
    strategy_id: i64,
    target_type: strategy_assignment::TargetType,
    target_id: Option<i64>,
) -> strategy_assignment::ActiveModel {
    strategy_assignment::ActiveModel {
        id: NotSet,
        strategy_id: Set(strategy_id),
        target_type: Set(target_type),
        target_id: Set(target_id),
    }
}

/// Only the columns every audit message carries; the rest arrives with later messages.
pub fn connection_row(
    device_rustdesk_id: &str,
    conn_id: i64,
    session_id: &str,
    guid: &str,
) -> connection_session::ActiveModel {
    connection_session::ActiveModel {
        id: NotSet,
        guid: Set(guid.to_owned()),
        device_rustdesk_id: Set(device_rustdesk_id.to_owned()),
        conn_id: Set(conn_id),
        session_id: Set(session_id.to_owned()),
        peer_rustdesk_id: Set(None),
        peer_name: Set(None),
        controller_user_id: Set(None),
        conn_type: Set(None),
        primary_auth: Set(None),
        two_factor: Set(None),
        ip: Set(None),
        started_at: Set(None),
        authorized_at: Set(None),
        ended_at: Set(None),
        note: Set(None),
        controller_note: Set(None),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn file_transfer_row(device_rustdesk_id: &str) -> file_transfer::ActiveModel {
    file_transfer::ActiveModel {
        id: NotSet,
        device_rustdesk_id: Set(device_rustdesk_id.to_owned()),
        peer_rustdesk_id: Set("987654321".to_owned()),
        conn_id: Set(7),
        direction: Set(file_transfer::Direction::Send),
        path: Set("/home/a/report.pdf".to_owned()),
        is_file: Set(true),
        file_count: Set(1),
        files: Set(r#"[["report.pdf",1024]]"#.to_owned()),
        ip: Set(Some("203.0.113.9".to_owned())),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn alarm_row(device_rustdesk_id: &str) -> alarm::ActiveModel {
    alarm::ActiveModel {
        id: NotSet,
        device_rustdesk_id: Set(device_rustdesk_id.to_owned()),
        typ: Set(2),
        info: Set(r#"{"ip":"203.0.113.9"}"#.to_owned()),
        conn_id: Set(7),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn login_event_row(user_id: Option<i64>, username: &str) -> login_event::ActiveModel {
    login_event::ActiveModel {
        id: NotSet,
        user_id: Set(user_id),
        username_attempted: Set(username.to_owned()),
        method: Set(login_event::Method::Password),
        result: Set("failure".to_owned()),
        client: Set(login_event::Client::Rustdesk),
        rustdesk_id: Set(Some("123456789".to_owned())),
        ip: Set(Some("203.0.113.9".to_owned())),
        created_at: Set(ts(T0_MS)),
    }
}

pub fn console_audit_row(actor_id: Option<i64>) -> console_audit::ActiveModel {
    console_audit::ActiveModel {
        id: NotSet,
        actor_kind: Set(console_audit::ActorKind::User),
        actor_id: Set(actor_id),
        action: Set("user.update".to_owned()),
        target_type: Set("user".to_owned()),
        target_id: Set("5".to_owned()),
        before: Set(None),
        after: Set(Some(r#"{"role":"admin"}"#.to_owned())),
        ip: Set(Some("203.0.113.9".to_owned())),
        created_at: Set(ts(T0_MS)),
    }
}
