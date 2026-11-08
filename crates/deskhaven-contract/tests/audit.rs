//! `POST /api/audit/{conn,file,alarm}` flows (03 §2.3-§2.6): what a sequence of records
//! leaves behind. The client only reads the status and whether the body is empty
//! (`src/server/connection.rs:1707-1742`); these tests read the audit tables.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_contract::app::TestApp;
use deskhaven_contract::check::Reply;
use deskhaven_contract::device_seeds::{DEVICE_ID, DEVICE_UUID_B64};
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::ErrorBody;
use deskhaven_contract::seeds::Seeds;
use deskhaven_db::entity::{alarm, audit_nonce, connection_session, file_transfer};
use sea_orm::{EntityTrait, PaginatorTrait, QueryOrder};
use serde_json::{Value, json};

const SESSION: u64 = 8_123_456_789_012_345_678;

fn post(path: &str, body: Value) -> Request {
    let mut request = Request::new("POST", path);
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(body);
    request
}

/// A record of the controlled device: `extra` plus the fields `post_conn_audit` adds
/// (`src/server/connection.rs:1559-1572`).
fn conn(extra: Value, conn_id: i64, session_id: Value, nonce: &str) -> Request {
    let mut body = json!({
        "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "conn_id": conn_id,
        "session_id": session_id, "nonce": nonce,
    });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    post("/api/audit/conn", body)
}

/// The three records of connection `conn_id`, as the client sends them: `new` before the
/// login request (session id still 0), the authorization and the close with the session id.
fn connection(conn_id: i64) -> [Request; 3] {
    let n = |kind: &str| format!("{conn_id}-{kind}");
    [
        conn(
            json!({ "ip": "198.51.100.4", "action": "new" }),
            conn_id,
            json!(0),
            &n("new"),
        ),
        conn(
            json!({ "peer": ["987654321", "alice@laptop"], "type": 1, "primary_auth": 3 }),
            conn_id,
            json!(SESSION),
            &n("auth"),
        ),
        conn(
            json!({ "action": "close" }),
            conn_id,
            json!(SESSION),
            &n("close"),
        ),
    ]
}

fn file(nonce: &str) -> Request {
    let info = json!({ "ip": "198.51.100.4", "name": "alice@laptop", "num": 1,
                       "files": [["", 1024]] });
    post(
        "/api/audit/file",
        json!({
            "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "peer_id": "987654321", "conn_id": 5,
            "type": 1, "path": "/tmp/report.pdf", "is_file": true,
            "info": info.to_string(), "nonce": nonce,
        }),
    )
}

fn alarm(nonce: &str) -> Request {
    post(
        "/api/audit/alarm",
        json!({
            "id": DEVICE_ID, "uuid": DEVICE_UUID_B64, "typ": 2, "conn_id": 5,
            "info": json!({ "ip": "198.51.100.4" }).to_string(), "nonce": nonce,
        }),
    )
}

async fn app() -> TestApp {
    let app = TestApp::start().await.unwrap();
    let names = vec!["registered-device".to_owned()];
    Seeds::builtin().apply(&names, &app.state).await.unwrap();
    app
}

async fn sessions(app: &TestApp) -> Vec<connection_session::Model> {
    connection_session::Entity::find()
        .order_by_asc(connection_session::Column::Id)
        .all(&app.state.db)
        .await
        .unwrap()
}

/// Stored, as the client sees it: 2xx with an empty body (anything in the body is a retry).
fn assert_stored(reply: &Reply) {
    assert_eq!(reply.status, 200);
    assert!(
        reply.body.is_empty(),
        "{:?}",
        String::from_utf8_lossy(&reply.body)
    );
}

#[tokio::test]
async fn three_records_arriving_out_of_order_merge_into_one_session() {
    // Arrange
    let app = app().await;
    let orders = [[2, 0, 1], [1, 2, 0], [2, 1, 0]];

    // Act
    for (i, order) in orders.iter().enumerate() {
        let records = connection(100 + i as i64);
        for &index in order {
            assert_stored(&app.send(&records[index]).await.unwrap());
        }
    }

    // Assert
    let rows = sessions(&app).await;
    assert_eq!(rows.len(), orders.len());
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.conn_id, 100 + i as i64);
        assert_eq!(row.session_id, SESSION.to_string());
        assert_eq!(row.ip.as_deref(), Some("198.51.100.4"));
        assert_eq!(row.peer_rustdesk_id.as_deref(), Some("987654321"));
        assert_eq!(
            (row.conn_type, row.primary_auth, row.two_factor),
            (Some(1), Some(3), None)
        );
        assert!(row.started_at.is_some() && row.authorized_at.is_some() && row.ended_at.is_some());
    }
}

#[tokio::test]
async fn a_session_id_sent_as_a_string_names_the_same_session() {
    // Arrange
    let app = app().await;
    let [_, auth, _] = connection(7);
    let close = conn(
        json!({ "action": "close" }),
        7,
        json!(SESSION.to_string()),
        "c",
    );

    // Act
    assert_stored(&app.send(&auth).await.unwrap());
    assert_stored(&app.send(&close).await.unwrap());

    // Assert
    let rows = sessions(&app).await;
    assert_eq!(rows.len(), 1);
    assert!(rows[0].ended_at.is_some());
}

#[tokio::test]
async fn a_retried_record_is_answered_200_empty_and_stored_once() {
    // Arrange
    let app = app().await;
    let records = [connection(7)[0].clone(), file("f"), alarm("a")];

    // Act
    for record in &records {
        assert_stored(&app.send(record).await.unwrap());
        assert_stored(&app.send(record).await.unwrap());
    }

    // Assert
    let db = &app.state.db;
    assert_eq!(sessions(&app).await.len(), 1);
    assert_eq!(file_transfer::Entity::find().count(db).await.unwrap(), 1);
    assert_eq!(alarm::Entity::find().count(db).await.unwrap(), 1);
    assert_eq!(audit_nonce::Entity::find().count(db).await.unwrap(), 3);
}

#[tokio::test]
async fn records_of_an_unknown_device_are_refused_with_404_and_store_nothing() {
    // Arrange
    let app = TestApp::start().await.unwrap();
    let records = [connection(7)[0].clone(), file("f"), alarm("a")];

    // Act
    let mut replies = Vec::new();
    for record in &records {
        replies.push(app.send(record).await.unwrap());
    }

    // Assert
    for reply in &replies {
        assert_eq!(reply.status, 404);
        assert_eq!(reply.json::<ErrorBody>().unwrap().error, "ID_NOT_FOUND");
    }
    let db = &app.state.db;
    assert!(sessions(&app).await.is_empty());
    assert_eq!(file_transfer::Entity::find().count(db).await.unwrap(), 0);
    assert_eq!(alarm::Entity::find().count(db).await.unwrap(), 0);
    assert_eq!(audit_nonce::Entity::find().count(db).await.unwrap(), 0);
}

/// The controller's note (`src/ui_session_interface.rs:2112-2115`) can come before the
/// controlled device's authorization record names the session; it waits for it.
#[tokio::test]
async fn a_controller_note_before_the_session_lands_on_it() {
    // Arrange
    let app = app().await;
    let note = post(
        "/api/audit/conn",
        json!({ "id": DEVICE_ID, "session_id": SESSION, "note": "printer driver" }),
    );
    let [new, auth, _] = connection(7);

    // Act
    assert_stored(&app.send(&new).await.unwrap());
    assert_stored(&app.send(&note).await.unwrap());
    assert_stored(&app.send(&auth).await.unwrap());

    // Assert
    let rows = sessions(&app).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].controller_note.as_deref(), Some("printer driver"));
}
