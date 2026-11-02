//! `login_events` rows written by `/api/login`: one per verified attempt (success, wrong
//! credentials, disabled, unverified, the failure that started a lockout), none for refused
//! or malformed requests, never a secret; a failed write never changes the reply.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_contract::app::{DEFAULT_PEER, TestApp};
use deskhaven_contract::requests::login;
use deskhaven_contract::seeds::{
    DISABLED_USERNAME, SEED_PASSWORD, SEED_RUSTDESK_ID, SEED_USERNAME, Seeds, UNKNOWN_USERNAME,
    UNVERIFIED_USERNAME,
};
use deskhaven_db::entity::login_event::{self, Client, Method};
use sea_orm::{ConnectionTrait, EntityTrait, QueryOrder};

const WRONG_PASSWORD: &str = "not-the-password";
const LOCKED_NAME: &str = "zed";

async fn app() -> TestApp {
    let app = TestApp::start().await.unwrap();
    let seeds = ["local-user", "disabled-user", "unverified-user"].map(str::to_owned);
    Seeds::builtin().apply(&seeds, &app.state).await.unwrap();
    app
}

async fn rows(app: &TestApp) -> Vec<login_event::Model> {
    login_event::Entity::find()
        .order_by_asc(login_event::Column::Id)
        .all(&app.state.db)
        .await
        .unwrap()
}

async fn status(app: &TestApp, username: &str, password: &str) -> u16 {
    app.send(&login(username, password)).await.unwrap().status
}

#[tokio::test]
async fn verified_attempts_are_recorded_and_refused_ones_are_not() {
    let app = app().await;
    let alice = app
        .state
        .users
        .get_by_username(SEED_USERNAME)
        .await
        .unwrap()
        .id;

    assert_eq!(status(&app, SEED_USERNAME, SEED_PASSWORD).await, 200);
    assert_eq!(status(&app, SEED_USERNAME, WRONG_PASSWORD).await, 401);
    assert_eq!(status(&app, UNKNOWN_USERNAME, WRONG_PASSWORD).await, 401);
    assert_eq!(status(&app, DISABLED_USERNAME, SEED_PASSWORD).await, 403);
    assert_eq!(status(&app, UNVERIFIED_USERNAME, SEED_PASSWORD).await, 403);
    for _ in 0..5 {
        assert_eq!(status(&app, LOCKED_NAME, WRONG_PASSWORD).await, 401);
    }
    assert_eq!(status(&app, LOCKED_NAME, WRONG_PASSWORD).await, 429);
    let mut not_json = login(SEED_USERNAME, SEED_PASSWORD);
    not_json
        .headers
        .insert("content-type".to_owned(), "text/plain".to_owned());
    assert_eq!(app.send(&not_json).await.unwrap().status, 400);

    let rows = rows(&app).await;
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r.username_attempted.as_str(), r.result.as_str(), r.user_id))
        .collect();
    let invalid = "invalid_credentials";
    assert_eq!(
        got,
        [
            (SEED_USERNAME, "success", Some(alice)),
            (SEED_USERNAME, invalid, None),
            (UNKNOWN_USERNAME, invalid, None),
            (DISABLED_USERNAME, "disabled", None),
            (UNVERIFIED_USERNAME, "unverified", None),
            (LOCKED_NAME, invalid, None),
            (LOCKED_NAME, invalid, None),
            (LOCKED_NAME, invalid, None),
            (LOCKED_NAME, invalid, None),
            (LOCKED_NAME, "locked", None),
        ]
    );
    for row in &rows {
        assert_eq!(row.method, Method::Password);
        assert_eq!(row.client, Client::Rustdesk);
        assert_eq!(row.rustdesk_id.as_deref(), Some(SEED_RUSTDESK_ID));
        assert_eq!(row.ip, Some(DEFAULT_PEER.ip().to_string()));
        let text = format!("{row:?}");
        for secret in [SEED_PASSWORD, WRONG_PASSWORD] {
            assert!(!text.contains(secret), "{text}");
        }
    }
}

#[tokio::test]
async fn login_still_answers_when_the_event_cannot_be_written() {
    let app = app().await;
    app.state
        .db
        .execute_unprepared("DROP TABLE login_events")
        .await
        .unwrap();

    assert_eq!(status(&app, SEED_USERNAME, SEED_PASSWORD).await, 200);
    assert_eq!(status(&app, SEED_USERNAME, WRONG_PASSWORD).await, 401);
    assert_eq!(status(&app, DISABLED_USERNAME, SEED_PASSWORD).await, 403);
}
