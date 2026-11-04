//! "My account" (M3-13), sessions: a user lists their client and console sessions and
//! revokes one or all others; never another user's. Revoking the current console session
//! signs out. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::login::set_cookies;
use common::own::{ME, REVOKE_OTHERS, SESSIONS};
use common::{Console, Harness, PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::sessions::{ClientDevice, SessionKind};
use deskhaven_core::users::{Password, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

/// A client session of `username` on device `rustdesk_id`; its id and bearer token.
async fn client_session(h: &Harness, username: &str, rustdesk_id: &str) -> (i64, String) {
    let auth = h
        .state
        .users
        .authenticate_local(username, &Password::new(PASSWORD))
        .await
        .unwrap();
    let device = ClientDevice {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: format!("uuid-{rustdesk_id}"),
        info: Some(json!({ "os": "linux", "name": "work laptop", "type": "client" })),
    };
    let (session, token) = h
        .state
        .sessions
        .create_client(auth.user.id, &auth.stamp, &device, None)
        .await
        .unwrap();
    (session.id, token.expose().to_owned())
}

async fn session_id_of(h: &Harness, console: &Console) -> i64 {
    let token = console.cookie.split_once('=').unwrap().1;
    let principal = h
        .state
        .sessions
        .authenticate(token, SessionKind::Console)
        .await
        .unwrap();
    principal.session_id
}

fn ids(list: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_i64().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

fn path(id: i64) -> String {
    format!("{SESSIONS}/{id}")
}

async fn is_alive(h: &Harness, token: &str) -> bool {
    let kind = SessionKind::Client;
    h.state.sessions.authenticate(token, kind).await.is_ok()
}

async fn audit_actions(db: &TestDb) -> Vec<(String, Option<i64>)> {
    let rows = console_audit::Entity::find()
        .filter(console_audit::Column::Action.like("session.%"))
        .all(&db.conn)
        .await
        .unwrap();
    rows.into_iter().map(|r| (r.action, r.actor_id)).collect()
}

#[tokio::test]
async fn the_list_has_the_users_client_and_console_sessions_only() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        let other = h.signed_in("alice").await;
        let (client_id, token) = client_session(&h, "alice", "111111111").await;
        let bob = h.signed_in("bob").await;
        client_session(&h, "bob", "222222222").await;
        let b = db.backend;

        let list = h.get(SESSIONS, &alice).await;

        assert_eq!(list.status, 200, "{b}: {:?}", list.body);
        let mine = [
            session_id_of(&h, &alice).await,
            session_id_of(&h, &other).await,
            client_id,
        ];
        let mut want = mine.to_vec();
        want.sort_unstable();
        assert_eq!(ids(&list.body), want, "{b}");
        let current: Vec<_> = list
            .body
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["current"] == true)
            .map(|s| s["id"].as_i64().unwrap())
            .collect();
        assert_eq!(current, vec![mine[0]], "{b}: exactly this session");
        let client = list
            .body
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == client_id)
            .unwrap();
        assert_eq!(client["kind"], "client", "{b}");
        assert_eq!(client["rustdesk_id"], "111111111", "{b}");
        assert_eq!(client["device_name"], "work laptop", "{b}");
        assert_eq!(client["device_os"], "linux", "{b}");
        let dump = list.body.to_string();
        assert!(!dump.contains(&token), "{b}: no token");
        assert!(!dump.contains("token_hash"), "{b}");
        assert_eq!(ids(&h.get(SESSIONS, &bob).await.body).len(), 2, "{b}");
    })
    .await;
}

#[tokio::test]
async fn revoking_one_session_never_reaches_another_users_session() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        let (mine, mine_token) = client_session(&h, "alice", "111111111").await;
        let (theirs, their_token) = client_session(&h, "bob", "222222222").await;
        let b = db.backend;

        let foreign = h
            .as_console(Method::DELETE, &path(theirs), &alice, None)
            .await;
        let unknown = h
            .as_console(Method::DELETE, &path(999_999), &alice, None)
            .await;
        let own = h
            .as_console(Method::DELETE, &path(mine), &alice, None)
            .await;
        let again = h
            .as_console(Method::DELETE, &path(mine), &alice, None)
            .await;

        assert_eq!(foreign.status, 404, "{b}: {:?}", foreign.body);
        assert_eq!(foreign.code(), "not_found", "{b}");
        assert_eq!(
            unknown.code(),
            "not_found",
            "{b}: same answer as a foreign id"
        );
        assert!(is_alive(&h, &their_token).await, "{b}: bob's session lives");
        assert_eq!(own.status, 204, "{b}");
        assert!(!is_alive(&h, &mine_token).await, "{b}");
        assert_eq!(again.status, 404, "{b}: already revoked");
        assert_eq!(h.get(ME, &alice).await.status, 200, "{b}: still signed in");
        let actions = audit_actions(db).await;
        assert_eq!(
            actions,
            vec![("session.revoke".to_owned(), Some(alice_id))],
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn revoking_the_current_session_signs_the_user_out() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let other = h.signed_in("alice").await;
        let current = session_id_of(&h, &alice).await;
        let b = db.backend;

        let gone = h
            .as_console(Method::DELETE, &path(current), &alice, None)
            .await;

        assert_eq!(gone.status, 204, "{b}");
        let cleared = set_cookies(&gone);
        assert_eq!(cleared.len(), 2, "{b}: {cleared:?}");
        assert!(cleared.iter().all(|c| c.contains("Max-Age=0")), "{b}");
        assert_eq!(h.get(ME, &alice).await.status, 401, "{b}");
        assert_eq!(
            h.get(ME, &other).await.status,
            200,
            "{b}: the other one stays"
        );
    })
    .await;
}

#[tokio::test]
async fn revoke_others_keeps_the_current_console_session_and_other_users() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        let other = h.signed_in("alice").await;
        let (_, token) = client_session(&h, "alice", "111111111").await;
        let bob = h.signed_in("bob").await;
        let (_, bob_token) = client_session(&h, "bob", "222222222").await;
        let b = db.backend;

        let done = h
            .as_console(Method::POST, REVOKE_OTHERS, &alice, Some(json!({})))
            .await;

        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["revoked"], 2, "{b}");
        assert_eq!(h.get(ME, &alice).await.status, 200, "{b}");
        assert_eq!(h.get(ME, &other).await.status, 401, "{b}");
        assert!(!is_alive(&h, &token).await, "{b}");
        assert_eq!(h.get(ME, &bob).await.status, 200, "{b}");
        assert!(is_alive(&h, &bob_token).await, "{b}");
        let actions = audit_actions(db).await;
        assert_eq!(
            actions,
            vec![("session.revoke_others".to_owned(), Some(alice_id))],
            "{b}"
        );
    })
    .await;
}
