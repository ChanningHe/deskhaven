//! An owner asking for updates must never make the test process contact GitHub.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_admin_api::SESSION_COOKIE;
use deskhaven_contract::app::{DEFAULT_PEER, TestApp, TestOptions};
use deskhaven_contract::fixture::{Listen, Request};
use deskhaven_core::users::{Credential, NewUser, Password, Role};
use serde_json::{Value, json};

const PASSWORD: &str = "owner-Password-for-updates-1";

async fn owner_cookie(app: &TestApp) -> String {
    let credential = Credential::Local {
        password: Password::new(PASSWORD),
    };
    let new = NewUser {
        role: Role::Owner,
        ..NewUser::new("update-owner", credential)
    };
    let id = app.state.users.create(new).await.unwrap().id;
    let auth = app
        .state
        .users
        .authenticate_local("update-owner", &Password::new(PASSWORD))
        .await
        .unwrap();
    let (_, token) = app
        .state
        .sessions
        .create_console(id, &auth.stamp, Some(DEFAULT_PEER.ip()), None)
        .await
        .unwrap();
    format!("{SESSION_COOKIE}={}", token.expose())
}

async fn update_check(app: &TestApp, cookie: &str) -> Value {
    let mut request = Request::new("GET", "/admin/api/v1/update-check");
    request
        .headers
        .insert("cookie".to_owned(), cookie.to_owned());
    let reply = app.send(&request).await.unwrap();
    assert_eq!(reply.status, 200);
    serde_json::from_slice(&reply.body).unwrap()
}

#[tokio::test]
async fn asking_for_updates_stays_in_the_process() {
    let app = TestApp::start_with_options(TestOptions {
        listen: Listen::Single,
        ..TestOptions::default()
    })
    .await
    .unwrap();
    let cookie = owner_cookie(&app).await;
    let nothing = json!({ "latest": null, "release_url": null });

    let first = update_check(&app, &cookie).await;
    for _ in 0..500 {
        if app.update_requests() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    let second = update_check(&app, &cookie).await;

    assert_eq!(app.update_requests(), 1);
    assert_eq!((first, second), (nothing.clone(), nothing));
}
