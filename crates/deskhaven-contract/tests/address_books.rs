//! Multi-step address book flows (03 §5.1): write then read back as the client does,
//! personal guid stability, shared passwords, tag renames reaching peers, refused writes
//! leaving the book unchanged, and books out of reach answering exactly like missing ones.
//! Every reply is decoded with the client mirror types.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_client_api::MAX_AB_BODY;
use deskhaven_contract::ab_seeds::{PERSONAL_PEER, SHARED_PASSWORD, SHARED_PEER};
use deskhaven_contract::app::TestApp;
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::{AbPeer, AbPersonal, AbTag, ErrorBody, HbbData, PagedResponse};
use deskhaven_contract::seeds::{SeedOutputs, Seeds};
use deskhaven_db::entity::address_book_peer;
use sea_orm::EntityTrait;
use serde_json::{Value, json};

/// An address book request as `ab_model.dart` sends it: bearer, JSON content type.
fn ab(method: &str, path: &str, token: &str, body: Option<Value>) -> Request {
    let mut request = Request::new(method, path);
    request
        .headers
        .insert("authorization".to_owned(), format!("Bearer {token}"));
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    if let Some(body) = body {
        request.body = Body::Json(body);
    }
    request
}

async fn app_with(seeds: &[&str]) -> (TestApp, SeedOutputs) {
    let app = TestApp::start().await.unwrap();
    let names: Vec<String> = seeds.iter().map(|s| (*s).to_owned()).collect();
    let outputs = Seeds::builtin().apply(&names, &app.state).await.unwrap();
    (app, outputs)
}

async fn send(app: &TestApp, request: Request) -> Reply {
    app.send(&request).await.unwrap()
}

async fn peers(app: &TestApp, token: &str, guid: &str) -> Vec<AbPeer> {
    let path = format!("/api/ab/peers?current=1&pageSize=100&ab={guid}");
    let reply = send(app, ab("POST", &path, token, None)).await;
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let HbbData(page) = reply.json::<HbbData<PagedResponse<AbPeer>>>().unwrap();
    page.data
}

fn raw_peers(reply: &Reply) -> Vec<Value> {
    reply.json::<Value>().unwrap()["data"]
        .as_array()
        .unwrap()
        .clone()
}

/// Asserts a write succeeded the way the client checks it: 200 and an empty body.
fn assert_done(reply: &Reply) {
    let body = String::from_utf8_lossy(&reply.body);
    assert_eq!((reply.status, body.as_ref()), (200, ""));
}

#[tokio::test]
async fn personal_guid_is_stable_and_keys_the_book() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let token = &out["session_token"];

    let first = send(&app, ab("POST", "/api/ab/personal", token, None)).await;
    let second = send(&app, ab("POST", "/api/ab/personal", token, None)).await;

    let HbbData(first) = first.json::<HbbData<AbPersonal>>().unwrap();
    let HbbData(second) = second.json::<HbbData<AbPersonal>>().unwrap();
    assert_eq!(first.guid, out["personal_guid"]);
    assert_eq!(second.guid, first.guid);
    let listed = peers(&app, token, &first.guid).await;
    assert_eq!(listed[0].id.as_deref(), Some(PERSONAL_PEER));
}

#[tokio::test]
async fn personal_add_then_read_keeps_the_hash_drops_the_password_and_echoes_extra() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let sent = json!({
        "id": "444444444", "hash": "h-444", "password": "must-not-be-kept", "alias": "lab",
        "tags": ["work"], "forceAlwaysRelay": "true", "rdpPort": "3390",
        "device_group_name": "client-sent", "same_server": true, "newClientField": [1, 2]
    });

    let added = send(
        &app,
        ab(
            "POST",
            &format!("/api/ab/peer/add/{guid}"),
            token,
            Some(sent),
        ),
    )
    .await;
    let update = json!({ "id": "444444444", "note": "n", "forceAlwaysRelay": "false" });
    let path = format!("/api/ab/peer/update/{guid}");
    let updated = send(&app, ab("PUT", &path, token, Some(update))).await;
    let path = format!("/api/ab/peers?current=1&pageSize=100&ab={guid}");
    let listed = send(&app, ab("POST", &path, token, None)).await;

    assert_done(&added);
    assert_done(&updated);
    let peer = raw_peers(&listed)
        .into_iter()
        .find(|p| p["id"] == "444444444")
        .unwrap();
    assert_eq!(peer["hash"], "h-444");
    assert_eq!(peer["forceAlwaysRelay"], "false");
    assert_eq!(peer["rdpPort"], "3390");
    assert_eq!(peer["note"], "n");
    assert_eq!(peer["newClientField"], json!([1, 2]));
    for absent in ["password", "device_group_name", "same_server"] {
        assert!(peer.get(absent).is_none(), "{absent} in {peer}");
    }
    let rows = address_book_peer::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    assert!(rows.iter().all(|r| r.password_enc.is_none()));
    assert!(rows.iter().all(|r| !r.extra.contains("must-not-be-kept")));
}

#[tokio::test]
async fn shared_passwords_round_trip_and_are_sealed_in_the_database() {
    let (app, out) = app_with(&["client-session", "ab-shared-write"]).await;
    let (token, guid) = (&out["session_token"], &out["write_guid"]);

    let change = json!({ "id": SHARED_PEER, "password": "rotated-pw", "hash": "ignored" });
    let path = format!("/api/ab/peer/update/{guid}");
    let updated = send(&app, ab("PUT", &path, token, Some(change))).await;

    assert_done(&updated);
    let listed = peers(&app, token, guid).await;
    assert_eq!(listed[0].password.as_deref(), Some("rotated-pw"));
    assert_eq!(listed[0].hash, None);
    let row = address_book_peer::Entity::find()
        .one(&app.state.db)
        .await
        .unwrap()
        .unwrap();
    let sealed = row.password_enc.unwrap();
    assert!(
        sealed.starts_with("v1:") && !sealed.contains("rotated-pw"),
        "{sealed}"
    );
    assert!(!sealed.contains(SHARED_PASSWORD));
    assert_eq!(row.hash, None);
}

#[tokio::test]
async fn a_renamed_tag_reaches_the_peers() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);

    let rename = json!({ "old": "work", "new": "office" });
    let path = format!("/api/ab/tag/rename/{guid}");
    let renamed = send(&app, ab("PUT", &path, token, Some(rename))).await;
    let tags = send(
        &app,
        ab("POST", &format!("/api/ab/tags/{guid}"), token, None),
    )
    .await;

    assert_done(&renamed);
    let tags = tags.json::<Vec<AbTag>>().unwrap();
    let names: Vec<_> = tags.iter().map(|t| t.name.as_deref()).collect();
    assert_eq!(names, [Some("office")]);
    let listed = peers(&app, token, guid).await;
    assert_eq!(listed[0].tags, Some(vec![json!("office")]));
}

#[tokio::test]
async fn refused_writes_leave_a_read_only_book_unchanged() {
    let (app, out) = app_with(&["client-session", "ab-shared-read"]).await;
    let (token, guid) = (&out["session_token"], &out["read_guid"]);
    let before = peers(&app, token, guid).await;

    let attempts = [
        ab(
            "POST",
            &format!("/api/ab/peer/add/{guid}"),
            token,
            Some(json!({"id": "9"})),
        ),
        ab(
            "PUT",
            &format!("/api/ab/peer/update/{guid}"),
            token,
            Some(json!({"id": SHARED_PEER, "alias": "x"})),
        ),
        ab(
            "DELETE",
            &format!("/api/ab/peer/{guid}"),
            token,
            Some(json!([SHARED_PEER])),
        ),
        ab(
            "DELETE",
            &format!("/api/ab/tag/{guid}"),
            token,
            Some(json!(["ops"])),
        ),
    ];
    for attempt in attempts {
        let reply = send(&app, attempt).await;
        assert_eq!(reply.status, 403);
        assert_eq!(
            reply.json::<ErrorBody>().unwrap().error,
            "Permission denied"
        );
    }

    let after = peers(&app, token, guid).await;
    assert_eq!(format!("{before:?}"), format!("{after:?}"));
}

#[tokio::test]
async fn unreachable_books_answer_byte_for_byte_like_missing_ones() {
    let (app, out) = app_with(&["client-session", "ab-foreign"]).await;
    let token = &out["session_token"];
    let guids = [
        out["foreign_personal_guid"].as_str(),
        out["foreign_shared_guid"].as_str(),
        "00000000-0000-7000-8000-000000000000",
    ];
    let requests = |guid: &str| {
        vec![
            ab(
                "POST",
                &format!("/api/ab/peers?current=1&pageSize=100&ab={guid}"),
                token,
                None,
            ),
            ab("POST", &format!("/api/ab/tags/{guid}"), token, None),
            ab(
                "POST",
                &format!("/api/ab/peer/add/{guid}"),
                token,
                Some(json!({"id": "1"})),
            ),
            ab(
                "PUT",
                &format!("/api/ab/tag/update/{guid}"),
                token,
                Some(json!({"name": "a", "color": 1})),
            ),
            ab(
                "DELETE",
                &format!("/api/ab/peer/{guid}"),
                token,
                Some(json!(["1"])),
            ),
        ]
    };

    let mut replies = Vec::new();
    for guid in guids {
        let mut per_guid = Vec::new();
        for request in requests(guid) {
            let reply = send(&app, request).await;
            per_guid.push((reply.status, reply.body));
        }
        replies.push(per_guid);
    }

    assert_eq!(replies[0], replies[2]);
    assert_eq!(replies[1], replies[2]);
    assert!(replies[2].iter().all(|(status, _)| *status == 404));
}

#[tokio::test]
async fn oversized_bodies_are_refused_before_parsing() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let huge = json!({ "id": "1", "note": "x".repeat(MAX_AB_BODY) });

    let reply = send(
        &app,
        ab(
            "POST",
            &format!("/api/ab/peer/add/{guid}"),
            token,
            Some(huge),
        ),
    )
    .await;

    assert_eq!(reply.status, 413);
    assert!(reply.json::<ErrorBody>().is_ok());
    assert_eq!(peers(&app, token, guid).await.len(), 1);
}
