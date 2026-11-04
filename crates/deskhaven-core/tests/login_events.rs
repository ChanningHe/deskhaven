//! Login event log (04 §7 `login_events`): one row per recorded attempt, the injected
//! clock's time, bounded attempted usernames, user deletion keeps the row. Every test runs
//! on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;

use common::{T0_MS, clock};
use deskhaven_core::login_events::{
    LoginClient, LoginEventLog, LoginMethod, LoginResult, MAX_USERNAME_ATTEMPTED_CHARS,
    NewLoginEvent,
};
use deskhaven_db::entity::{login_event, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder};

fn event<'a>(username: &'a str, result: LoginResult, user_id: Option<i64>) -> NewLoginEvent<'a> {
    NewLoginEvent {
        user_id,
        username_attempted: username,
        method: LoginMethod::Password,
        result,
        client: LoginClient::Rustdesk,
        rustdesk_id: Some("123456789"),
        ip: Some("2001:db8::7".parse::<IpAddr>().unwrap()),
    }
}

async fn rows(db: &TestDb) -> Vec<login_event::Model> {
    login_event::Entity::find()
        .order_by_asc(login_event::Column::Id)
        .all(&db.conn)
        .await
        .unwrap()
}

async fn insert_user(db: &TestDb, username: &str) -> i64 {
    let now = time::from_millis(T0_MS).unwrap();
    let row = user::ActiveModel {
        username: Set(username.to_owned()),
        username_norm: Set(username.to_owned()),
        display_name: Set(username.to_owned()),
        email_verified: Set(false),
        auth_source: Set(user::AuthSource::Local),
        role: Set(user::Role::User),
        status: Set(user::Status::Active),
        must_change_password: Set(false),
        note: Set(String::new()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    row.insert(&db.conn).await.unwrap().id
}

#[tokio::test]
async fn records_every_field_at_the_clock_time() {
    each_migrated_backend(async |db: &TestDb| {
        let log = LoginEventLog::new(db.conn.clone(), clock());
        let alice = insert_user(db, "alice").await;

        log.record(&event("alice", LoginResult::Success, Some(alice)))
            .await
            .unwrap();
        log.record(&event("nobody", LoginResult::InvalidCredentials, None))
            .await
            .unwrap();

        let rows = rows(db).await;
        assert_eq!(rows.len(), 2, "{}", db.backend);
        let first = &rows[0];
        assert_eq!(first.user_id, Some(alice));
        assert_eq!(first.username_attempted, "alice");
        assert_eq!(first.method, login_event::Method::Password);
        assert_eq!(first.result, "success");
        assert_eq!(first.client, login_event::Client::Rustdesk);
        assert_eq!(first.rustdesk_id.as_deref(), Some("123456789"));
        assert_eq!(first.ip.as_deref(), Some("2001:db8::7"));
        assert_eq!(first.created_at.timestamp_millis(), T0_MS);
        assert_eq!(
            (rows[1].user_id, rows[1].result.as_str()),
            (None, "invalid_credentials")
        );
    })
    .await;
}

#[tokio::test]
async fn result_labels_are_stable() {
    let labels = [
        (LoginResult::Success, "success"),
        (LoginResult::InvalidCredentials, "invalid_credentials"),
        (LoginResult::Disabled, "disabled"),
        (LoginResult::Unverified, "unverified"),
        (LoginResult::Locked, "locked"),
        (LoginResult::SecondFactorRequired, "tfa_required"),
        (LoginResult::SecondFactorSetupRequired, "tfa_setup_required"),
        (LoginResult::InvalidCode, "invalid_code"),
        (LoginResult::InvalidChallenge, "invalid_challenge"),
    ];
    for (result, label) in labels {
        assert_eq!(result.as_str(), label);
    }
}

#[tokio::test]
async fn client_supplied_text_is_stored_single_line_without_invisible_characters() {
    each_migrated_backend(async |db: &TestDb| {
        let log = LoginEventLog::new(db.conn.clone(), clock());
        // Bidi override, zero-width space, newline, NUL and a bell: none may reach the log.
        let nasty = "ad\u{202E}min\u{200B}\nx\0y\u{7}";
        let mut login = event(nasty, LoginResult::InvalidCredentials, None);
        login.rustdesk_id = Some("12\u{202E}34\n5\0");

        log.record(&login).await.unwrap();

        let row = &rows(db).await[0];
        assert_eq!(row.username_attempted, "adminxy", "{}", db.backend);
        assert_eq!(row.rustdesk_id.as_deref(), Some("12345"), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn attempted_usernames_are_truncated() {
    each_migrated_backend(async |db: &TestDb| {
        let log = LoginEventLog::new(db.conn.clone(), clock());
        let long = "é".repeat(MAX_USERNAME_ATTEMPTED_CHARS + 50);

        log.record(&event(&long, LoginResult::InvalidCredentials, None))
            .await
            .unwrap();

        let stored = &rows(db).await[0].username_attempted;
        assert_eq!(
            stored.chars().count(),
            MAX_USERNAME_ATTEMPTED_CHARS,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn deleting_the_user_keeps_the_event() {
    each_migrated_backend(async |db: &TestDb| {
        let log = LoginEventLog::new(db.conn.clone(), clock());
        let bob = insert_user(db, "bob").await;
        log.record(&event("bob", LoginResult::Success, Some(bob)))
            .await
            .unwrap();

        user::Entity::delete_by_id(bob)
            .exec(&db.conn)
            .await
            .unwrap();

        let rows = rows(db).await;
        assert_eq!(rows.len(), 1, "{}", db.backend);
        assert_eq!(
            (rows[0].user_id, rows[0].username_attempted.as_str()),
            (None, "bob")
        );
    })
    .await;
}
