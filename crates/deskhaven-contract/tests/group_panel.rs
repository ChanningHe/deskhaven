//! The group panel endpoints (03 §4) beyond single fixtures: the visibility scope follows
//! group membership and ignores roles (06 §4.2), and paging walks the whole visible set
//! the way the client's loop does. Every reply is decoded with the client mirror types.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_contract::app::TestApp;
use deskhaven_contract::fixture::Request;
use deskhaven_contract::mirror::{ClientUserPayload, PagedResponse, PeerPayload};
use deskhaven_contract::seeds::{SEED_USERNAME, SeedOutputs, Seeds};
use deskhaven_core::groups::NewGroup;
use deskhaven_core::users::{Credential, NewUser, Role};

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
