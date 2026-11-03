//! `/api/audit/*` replies other than "stored": body limit, malformed bodies, rate limits,
//! the controller note's buffer, and leniency towards newer clients. What the client does
//! with each status: `src/server/connection.rs:1711-1746` (4xx but 408 / 429: dropped; 429
//! and 5xx: retried).
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use deskhaven_client_api::{MAX_AUDIT_BODY, TRY_LATER_MESSAGE};
use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::device_seeds::{DEVICE_ID, DEVICE_UUID_B64};
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::ErrorBody;
use deskhaven_contract::seeds::Seeds;
use deskhaven_core::audit::{
    AUDIT_PER_DEVICE_PER_MINUTE, AUDIT_PER_IP_PER_MINUTE, MAX_NOTES_PER_SOURCE_DEVICE,
};
use deskhaven_core::clock::{Clock, ManualClock, SystemClock};
use deskhaven_db::entity::{connection_session, file_transfer};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

fn post(path: &str, body: Body) -> Request {
    let mut request = Request::new("POST", path);
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = body;
    request
}

fn alarm(nonce: &str) -> Request {
    let body = json!({ "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "typ": 2, "conn_id": 5,
                       "info": "{\"ip\":\"198.51.100.4\"}", "nonce": nonce });
    post("/api/audit/alarm", Body::Json(body))
}

fn note(session_id: u64, id: &str) -> Request {
    let body = json!({ "id": id, "session_id": session_id, "note": "n" });
    post("/api/audit/conn", Body::Json(body))
}

/// On a clock that stands still, so no bucket refills while a test spends it.
async fn app() -> TestApp {
    let clock = Arc::new(ManualClock::at(SystemClock.now_ms()));
    let options = TestOptions {
        clock: Some(clock),
        ..TestOptions::default()
    };
    let app = TestApp::start_with_options(options).await.unwrap();
    let names = vec!["registered-device".to_owned()];
    Seeds::builtin().apply(&names, &app.state).await.unwrap();
    app
}

fn assert_try_later(reply: &Reply, status: u16) {
    assert_eq!(reply.status, status);
    let wait = reply.headers.get("retry-after").unwrap().to_str().unwrap();
    assert!(wait.parse::<u64>().unwrap() >= 1, "{wait}");
    assert_eq!(reply.json::<ErrorBody>().unwrap().error, TRY_LATER_MESSAGE);
}

#[tokio::test]
async fn an_oversized_body_is_413_and_a_malformed_one_400() {
    // Arrange
    let app = app().await;
    let huge = post(
        "/api/audit/file",
        Body::Text("x".repeat(MAX_AUDIT_BODY + 1)),
    );
    let malformed = post("/api/audit/alarm", Body::Text("{not json".to_owned()));

    // Act
    let too_large = app.send(&huge).await.unwrap();
    let invalid = app.send(&malformed).await.unwrap();

    // Assert
    assert_eq!(too_large.status, 413);
    assert_eq!(invalid.status, 400);
    assert!(!invalid.json::<ErrorBody>().unwrap().error.is_empty());
}

/// Every request counts against the source network, malformed ones included.
#[tokio::test]
async fn a_source_over_its_quota_gets_429_with_retry_after() {
    // Arrange
    let app = app().await;
    let garbage = post("/api/audit/conn", Body::Text("[]".to_owned()));
    for _ in 0..AUDIT_PER_IP_PER_MINUTE {
        assert_eq!(app.send(&garbage).await.unwrap().status, 400);
    }

    // Act
    let reply = app.send(&alarm("over")).await.unwrap();

    // Assert
    assert_try_later(&reply, 429);
}

#[tokio::test]
async fn a_device_over_its_quota_gets_429_with_retry_after() {
    // Arrange
    let app = app().await;
    for i in 0..AUDIT_PER_DEVICE_PER_MINUTE {
        let reply = app.send(&alarm(&format!("n{i}"))).await.unwrap();
        assert_eq!((reply.status, reply.body.len()), (200, 0));
    }

    // Act
    let reply = app.send(&alarm("over")).await.unwrap();

    // Assert
    assert_try_later(&reply, 429);
}

/// The note carries no uuid: its reply must not tell whether an id is registered.
#[tokio::test]
async fn a_note_for_an_unknown_id_is_answered_like_any_other() {
    // Arrange
    let app = app().await;

    // Act
    let unknown = app.send(&note(7, "555555555")).await.unwrap();
    let known = app.send(&note(7, DEVICE_ID)).await.unwrap();

    // Assert
    assert_eq!((unknown.status, unknown.body.len()), (200, 0));
    assert_eq!((known.status, known.body.len()), (200, 0));
}

/// A note that finds no room to wait is dropped, but answered like any other note: a
/// different reply would tell a prober which ids are registered (a full slot needs one).
#[tokio::test]
async fn a_note_that_finds_no_room_is_dropped_but_answered_200() {
    // Arrange
    let app = app().await;
    for session in 1..=MAX_NOTES_PER_SOURCE_DEVICE as u64 {
        let reply = app.send(&note(session, DEVICE_ID)).await.unwrap();
        assert_eq!((reply.status, reply.body.len()), (200, 0));
    }

    // Act
    let reply = app.send(&note(1_000, DEVICE_ID)).await.unwrap();
    let auth = json!({ "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "conn_id": 5,
                       "session_id": 1_000, "nonce": "auth", "peer": ["987654321", "bob"],
                       "type": 0 });
    let stored = app
        .send(&post("/api/audit/conn", Body::Json(auth)))
        .await
        .unwrap();

    // Assert
    assert_eq!((reply.status, reply.body.len()), (200, 0));
    assert_eq!(stored.status, 200);
    let rows = connection_session::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    assert_eq!(rows[0].controller_note, None, "the note was not kept");
}

/// A newer client may add elements to a `[name, size]` pair; the record is still stored.
#[tokio::test]
async fn a_file_record_with_longer_tuples_is_stored() {
    // Arrange
    let app = app().await;
    let info = json!({ "ip": "198.51.100.4", "num": 1, "files": [["a.txt", 5, "extra", 1]] });
    let body: Value = json!({
        "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "peer_id": "987654321", "conn_id": 5,
        "type": 0, "path": "/tmp", "is_file": false, "info": info.to_string(), "nonce": "f",
    });

    // Act
    let reply = app
        .send(&post("/api/audit/file", Body::Json(body)))
        .await
        .unwrap();

    // Assert
    assert_eq!((reply.status, reply.body.len()), (200, 0));
    let rows = file_transfer::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    assert_eq!(rows[0].files, "[[\"a.txt\",5]]");
}
