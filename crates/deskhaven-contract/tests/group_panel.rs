//! The group panel endpoints (03 §4) beyond single fixtures: the visibility scope follows
//! group membership and ignores roles (06 §4.2), and paging walks the whole visible set
//! the way the client's loop does. Every reply is decoded with the client mirror types.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::fixture::Request;
use deskhaven_contract::mirror::{ClientUserPayload, PagedResponse, PeerPayload};
use deskhaven_contract::seeds::{SEED_USERNAME, SeedOutputs, Seeds};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::groups::NewGroup;
use deskhaven_core::users::{Credential, NewUser, Role, Status};
use serde_json::{Value, json};

const ALICE_DEVICE: &str = "700000001";
const TEAMMATE_DEVICE: &str = "700000002";
const DISABLED_TEAMMATE_DEVICE: &str = "700000003";
const STRANGER_DEVICE: &str = "700000004";

async fn scoped_app() -> (TestApp, SeedOutputs) {
    let app = TestApp::start().await.unwrap();
    let names = ["client-session", "group-scope"].map(str::to_owned);
    let outputs = Seeds::builtin().apply(&names, &app.state).await.unwrap();
    (app, outputs)
}

async fn get_page<T: serde::de::DeserializeOwned>(
    app: &TestApp,
    token: &str,
    path: &str,
) -> PagedResponse<T> {
    let mut request = Request::new("GET", path);
    let bearer = format!("Bearer {token}");
    request.headers.insert("authorization".to_owned(), bearer);
    let reply = app.send(&request).await.unwrap();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json::<PagedResponse<T>>().unwrap()
}

async fn peer_ids(app: &TestApp, token: &str) -> Vec<String> {
    let path = "/api/peers?current=1&pageSize=100&accessible=&status=1";
    let page = get_page::<PeerPayload>(app, token, path).await;
    page.data.into_iter().map(|peer| peer.id).collect()
}

async fn user_names(app: &TestApp, token: &str) -> Vec<String> {
    let path = "/api/users?current=1&pageSize=100&accessible=&status=1";
    let page = get_page::<ClientUserPayload>(app, token, path).await;
    page.data.into_iter().map(|user| user.name).collect()
}

async fn group_id(app: &TestApp, name: &str) -> i64 {
    let groups = app.state.groups.list().await.unwrap();
    groups.iter().find(|g| g.name == name).unwrap().id
}

async fn alice_id(app: &TestApp) -> i64 {
    app.state
        .users
        .get_by_username(SEED_USERNAME)
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn an_admin_sees_the_same_scope_as_an_ordinary_user() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    let before = peer_ids(&app, token).await;

    app.state
        .users
        .set_role(alice_id(&app).await, Role::Admin)
        .await
        .unwrap();
    let after = peer_ids(&app, token).await;

    assert_eq!(
        before,
        [ALICE_DEVICE, TEAMMATE_DEVICE, DISABLED_TEAMMATE_DEVICE]
    );
    assert_eq!(after, before);
    assert_eq!(user_names(&app, token).await, ["alice", "tess"]);
}

#[tokio::test]
async fn leaving_the_shared_group_hides_the_teammates_at_once() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    let team = group_id(&app, "Team").await;

    app.state
        .groups
        .remove_member(team, alice_id(&app).await)
        .await
        .unwrap();

    assert_eq!(peer_ids(&app, token).await, [ALICE_DEVICE]);
    assert_eq!(user_names(&app, token).await, ["alice"]);
}

#[tokio::test]
async fn joining_another_group_adds_its_members_devices() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    let elsewhere = group_id(&app, "Elsewhere").await;

    app.state
        .groups
        .add_member(elsewhere, alice_id(&app).await)
        .await
        .unwrap();

    let ids = peer_ids(&app, token).await;
    assert!(ids.contains(&STRANGER_DEVICE.to_owned()), "{ids:?}");
    assert_eq!(user_names(&app, token).await, ["alice", "sam", "tess"]);
}

#[tokio::test]
async fn sharing_two_groups_lists_a_teammate_once() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    let tess = app.state.users.get_by_username("tess").await.unwrap().id;
    let second = app
        .state
        .groups
        .create(NewGroup::new("Second"))
        .await
        .unwrap();

    for member in [alice_id(&app).await, tess] {
        app.state
            .groups
            .add_member(second.id, member)
            .await
            .unwrap();
    }

    assert_eq!(user_names(&app, token).await, ["alice", "tess"]);
    let ids = peer_ids(&app, token).await;
    assert_eq!(
        ids,
        [ALICE_DEVICE, TEAMMATE_DEVICE, DISABLED_TEAMMATE_DEVICE]
    );
}

#[tokio::test]
async fn walking_the_pages_visits_every_user_once_with_a_constant_total() {
    const TEAMMATES: usize = 30;
    const PAGE_SIZE: usize = 7;
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    let team = group_id(&app, "Team").await;
    for i in 0..TEAMMATES {
        let new = NewUser::new(format!("mate{i:02}"), Credential::Oidc);
        let user = app.state.users.create(new).await.unwrap();
        app.state.groups.add_member(team, user.id).await.unwrap();
    }

    let mut seen = Vec::new();
    let mut totals = Vec::new();
    let mut current = 1;
    loop {
        let path = format!("/api/users?current={current}&pageSize={PAGE_SIZE}&status=1");
        let page = get_page::<ClientUserPayload>(&app, token, &path).await;
        totals.push(page.total);
        seen.extend(page.data.into_iter().map(|u| u.name));
        if current * PAGE_SIZE >= usize::try_from(page.total).unwrap() {
            break;
        }
        current += 1;
    }

    let mut sorted = seen.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seen, sorted, "pages overlap or are out of order");
    assert_eq!(seen.len(), TEAMMATES + 2);
    assert!(totals.iter().all(|t| *t == totals[0]), "{totals:?}");
}

const ENDPOINTS: [&str; 3] = [
    "/api/users?current=1&pageSize=100&accessible=&status=1",
    "/api/peers?current=1&pageSize=100&accessible=&status=1",
    "/api/device-group/accessible?current=1&pageSize=100",
];

async fn status_of(app: &TestApp, token: &str, path: &str) -> (u16, String) {
    let mut request = Request::new("GET", path);
    let bearer = format!("Bearer {token}");
    request.headers.insert("authorization".to_owned(), bearer);
    let reply = app.send(&request).await.unwrap();
    let body = String::from_utf8_lossy(&reply.body).into_owned();
    (reply.status, body)
}

async fn assert_all_unauthorized(app: &TestApp, token: &str) {
    for path in ENDPOINTS {
        let (status, body) = status_of(app, token, path).await;
        assert_eq!(status, 401, "{path}: {body}");
        assert_eq!(body, r#"{"error":"Unauthorized"}"#, "{path}");
    }
}

#[tokio::test]
async fn other_users_rows_carry_neither_note_nor_email_but_the_own_row_does() {
    let (app, out) = scoped_app().await;
    let path = "/api/users?current=1&pageSize=100";

    let page = get_page::<Value>(&app, &out["session_token"], path).await;

    let names: Vec<_> = page
        .data
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alice", "tess", "tom", "uma"]);
    for row in &page.data {
        if row["name"] == "alice" {
            assert_eq!(row["email"], "alice@example.com");
            assert_eq!(row["note"], "admin note on alice");
        } else {
            assert_eq!((&row["email"], &row["note"]), (&Value::Null, &json!("")));
            assert!(row["status"].is_i64() && row["info"].is_object(), "{row}");
        }
    }
}

#[tokio::test]
async fn disabling_an_account_does_not_hide_its_devices() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];

    let ids = peer_ids(&app, token).await;

    assert!(
        ids.contains(&DISABLED_TEAMMATE_DEVICE.to_owned()),
        "{ids:?}"
    );
    let path = "/api/users?current=1&pageSize=100&status=-1";
    let unverified = get_page::<ClientUserPayload>(&app, token, path).await;
    let names: Vec<_> = unverified.data.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["uma"]);
}

#[tokio::test]
async fn a_revoked_session_is_a_401_on_every_endpoint() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    assert_eq!(status_of(&app, token, ENDPOINTS[0]).await.0, 200);

    assert!(app.state.sessions.revoke_token(token).await.unwrap());

    assert_all_unauthorized(&app, token).await;
}

#[tokio::test]
async fn an_expired_session_is_a_401_on_every_endpoint() {
    const PAST_THE_SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 400);
    let clock = Arc::new(ManualClock::at(1_730_000_000_000));
    let options = TestOptions {
        clock: Some(clock.clone()),
        ..TestOptions::default()
    };
    let app = TestApp::start_with_options(options).await.unwrap();
    let names = ["client-session", "group-scope"].map(str::to_owned);
    let out = Seeds::builtin().apply(&names, &app.state).await.unwrap();
    let token = &out["session_token"];
    assert_eq!(status_of(&app, token, ENDPOINTS[0]).await.0, 200);

    clock.advance(PAST_THE_SESSION_TTL);

    assert_all_unauthorized(&app, token).await;
}

#[tokio::test]
async fn a_disabled_user_is_a_401_on_every_endpoint() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];
    assert_eq!(status_of(&app, token, ENDPOINTS[0]).await.0, 200);

    let alice = alice_id(&app).await;
    app.state
        .users
        .set_status(alice, Status::Disabled)
        .await
        .unwrap();

    assert_all_unauthorized(&app, token).await;
}

#[tokio::test]
async fn refused_paging_names_the_clients_parameters() {
    let (app, out) = scoped_app().await;
    let token = &out["session_token"];

    let (size, size_body) = status_of(&app, token, "/api/peers?current=1&pageSize=1000").await;
    let (page, page_body) = status_of(&app, token, "/api/users?current=0&pageSize=10").await;

    assert_eq!((size, page), (400, 400));
    assert!(size_body.contains("pageSize"), "{size_body}");
    assert!(page_body.contains("current"), "{page_body}");
}
