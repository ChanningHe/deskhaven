//! Console user administration end to end, through the real single-listener server:
//! an owner creates a user and puts it in a group via the admin API, the user logs in with
//! the official client's request, the owner revokes its sessions, and the client's token
//! is refused (401) on its next call. (The browser version of this flow is M3-15's
//! Playwright suite.)
#![allow(clippy::unwrap_used)]

use deskhaven_admin_api::{SESSION_COOKIE, csrf_token};
use deskhaven_contract::app::{DEFAULT_PEER, TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::{Body, Listen, Request};
use deskhaven_contract::mirror::{ClientUserPayload, LoginResponse};
use deskhaven_contract::requests::{bearer, login};
use deskhaven_core::users::{Credential, NewUser, Password, Role};
use serde_json::{Value, json};

const OWNER_PASSWORD: &str = "owner-Password-for-flow-1";
const USER_PASSWORD: &str = "flow-User-password-2";

struct Owner {
    cookie: String,
    csrf: String,
}

async fn owner(app: &TestApp) -> Owner {
    let credential = Credential::Local {
        password: Password::new(OWNER_PASSWORD),
    };
    let new = NewUser {
        role: Role::Owner,
        ..NewUser::new("flow-owner", credential)
    };
    let id = app.state.users.create(new).await.unwrap().id;
    let auth = app
        .state
        .users
        .authenticate_local("flow-owner", &Password::new(OWNER_PASSWORD))
        .await
        .unwrap();
    let (_, token) = app
        .state
        .sessions
        .create_console(id, &auth.stamp, Some(DEFAULT_PEER.ip()), None)
        .await
        .unwrap();
    Owner {
        cookie: format!("{SESSION_COOKIE}={}", token.expose()),
        csrf: csrf_token(token.expose()),
    }
}

/// An admin API request as the console sends it.
fn console(method: &str, path: &str, owner: &Owner, body: Option<Value>) -> Request {
    let mut request = Request::new(method, path);
    request
        .headers
        .insert("cookie".to_owned(), owner.cookie.clone());
    request
        .headers
        .insert("x-deskhaven-csrf".to_owned(), owner.csrf.clone());
    if let Some(body) = body {
        request
            .headers
            .insert("content-type".to_owned(), "application/json".to_owned());
        request.body = Body::Json(body);
    }
    request
}

fn json_of(reply: &Reply) -> Value {
    serde_json::from_slice(&reply.body).unwrap()
}

#[tokio::test]
async fn revoking_a_users_sessions_in_the_console_logs_its_client_out() {
    // Arrange: the real server on one listener, an owner signed in to the console.
    let app = TestApp::start_with_options(TestOptions {
        listen: Listen::Single,
        ..TestOptions::default()
    })
    .await
    .unwrap();
    let owner = owner(&app).await;

    // Act 1: create the user and a group, add the user to it.
    let body = json!({ "username": "flow-user", "password": USER_PASSWORD, "must_change_password": false });
    let created = app
        .send(&console("POST", "/admin/api/v1/users", &owner, Some(body)))
        .await
        .unwrap();
    assert_eq!(
        created.status,
        201,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let user_id = json_of(&created)["user"]["id"].as_i64().unwrap();
    let group = app
        .send(&console(
            "POST",
            "/admin/api/v1/groups",
            &owner,
            Some(json!({ "name": "flow" })),
        ))
        .await
        .unwrap();
    let group_id = json_of(&group)["id"].as_i64().unwrap();
    let members = format!("/admin/api/v1/groups/{group_id}/members");
    let added = app
        .send(&console(
            "POST",
            &members,
            &owner,
            Some(json!({ "user_id": user_id })),
        ))
        .await
        .unwrap();
    assert_eq!(added.status, 204);

    // Act 2: the user logs in from a client and uses its token.
    let reply = app.send(&login("flow-user", USER_PASSWORD)).await.unwrap();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let token = reply.json::<LoginResponse>().unwrap().access_token.unwrap();
    let before = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
    assert_eq!(before.status, 200);
    assert_eq!(
        before.json::<ClientUserPayload>().unwrap().name,
        "flow-user"
    );

    // Act 3: the owner revokes every session of the user.
    let path = format!("/admin/api/v1/users/{user_id}/revoke-sessions");
    let revoked = app
        .send(&console("POST", &path, &owner, None))
        .await
        .unwrap();

    // Assert: one session revoked; the client's next call is a 401.
    assert_eq!(revoked.status, 200);
    assert_eq!(json_of(&revoked), json!({ "revoked": 1 }));
    let after = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
    assert_eq!(after.status, 401);
    let listed = app
        .send(&console("GET", &format!("{members}?q=flow"), &owner, None))
        .await
        .unwrap();
    assert_eq!(json_of(&listed)["items"][0]["username"], "flow-user");
}
