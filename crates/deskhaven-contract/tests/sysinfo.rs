//! `POST /api/sysinfo` flows (03 §2.2): what the replies of the fixtures leave behind.
//! The client only reads the text; these tests read the registry through the app's state.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_client_api::TRY_LATER_MESSAGE;
use deskhaven_contract::app::TestApp;
use deskhaven_contract::device_seeds::{
    DEVICE_ID, DEVICE_UUID_B64, PRESET_BOOK, PRESET_DEVICE_GROUP, PRESET_STRATEGY, PRESET_USER,
};
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::ErrorBody;
use deskhaven_contract::requests::from;
use deskhaven_contract::seeds::Seeds;
use deskhaven_core::addressbooks::{Actor, AddressBookService};
use deskhaven_core::devices::{DeviceRepository, SYSINFO_PER_IP_PER_MINUTE};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::strategies::Target;
use deskhaven_db::entity::device_group;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

/// The body `sync.rs` builds (get_sysinfo + version, id, uuid), plus `extra` keys.
fn sysinfo_request(hostname: &str, extra: Value) -> Request {
    let mut body = json!({
        "cpu": "8/4 cores", "memory": "15.5GB", "os": "windows / Windows 11 Pro - 10.0.22631",
        "hostname": hostname, "username": "frontdesk", "version": "1.4.1",
        "id": DEVICE_ID, "uuid": DEVICE_UUID_B64,
    });
    if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
        body.extend(extra);
    }
    let mut request = Request::new("POST", "/api/sysinfo");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(body);
    request
}

async fn app_with(seeds: &[&str]) -> TestApp {
    let app = TestApp::start().await.unwrap();
    let names: Vec<String> = seeds.iter().map(|s| (*s).to_owned()).collect();
    Seeds::builtin().apply(&names, &app.state).await.unwrap();
    app
}

fn text(reply: &deskhaven_contract::check::Reply) -> &str {
    std::str::from_utf8(&reply.body).unwrap()
}

fn every_preset() -> Value {
    json!({
        "preset-address-book-name": PRESET_BOOK, "preset-address-book-tag": "printers",
        "preset-address-book-alias": "Reception", "preset-address-book-password": "s3cret-pass",
        "preset-user-name": PRESET_USER, "preset-strategy-name": PRESET_STRATEGY,
        "preset-device-group-name": PRESET_DEVICE_GROUP, "preset-note": "front desk",
    })
}

/// `devices.accept_presets` is off by default: presets are self-reported, so the reply is
/// the same and nothing is placed.
#[tokio::test]
async fn presets_are_ignored_unless_accepted() {
    // Arrange
    let app = app_with(&["registered-device", "device-preset-targets"]).await;

    // Act
    let reply = app
        .send(&sysinfo_request("reception-01", every_preset()))
        .await
        .unwrap();

    // Assert
    assert_eq!((reply.status, text(&reply)), (200, "SYSINFO_UPDATED"));
    let row = DeviceRepository::new(app.state.db.clone())
        .find_by_rustdesk_id(DEVICE_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.hostname.as_deref(), Some("reception-01"));
    let placed = (row.owner_user_id, row.device_group_id, row.note);
    assert_eq!(placed, (None, None, None));
    let assigned = app.state.strategies.assignment(Target::Device(row.id));
    assert_eq!(assigned.await.unwrap(), None);
}

#[tokio::test]
async fn presets_of_the_first_sysinfo_place_the_device() {
    // Arrange
    let seeds = [
        "registered-device",
        "device-preset-targets",
        "device-presets-accepted",
    ];
    let app = app_with(&seeds).await;
    let presets = every_preset();

    // Act
    let reply = app
        .send(&sysinfo_request("reception-01", presets))
        .await
        .unwrap();

    // Assert
    assert_eq!((reply.status, text(&reply)), (200, "SYSINFO_UPDATED"));
    let state = &app.state;
    let row = DeviceRepository::new(state.db.clone())
        .find_by_rustdesk_id(DEVICE_ID)
        .await
        .unwrap()
        .unwrap();
    let owner = state.users.get_by_username(PRESET_USER).await.unwrap().id;
    let group = device_group::Entity::find()
        .filter(device_group::Column::Name.eq(PRESET_DEVICE_GROUP))
        .one(&state.db)
        .await
        .unwrap()
        .unwrap()
        .id;
    let strategy = state
        .strategies
        .get_by_name(PRESET_STRATEGY)
        .await
        .unwrap()
        .id;
    assert_eq!(row.hostname.as_deref(), Some("reception-01"));
    assert_eq!(row.owner_user_id, Some(owner));
    assert_eq!(row.device_group_id, Some(group));
    assert_eq!(row.note.as_deref(), Some("front desk"));
    let assigned = state.strategies.assignment(Target::Device(row.id)).await;
    assert_eq!(assigned.unwrap(), Some(strategy));
    let books = AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        state.crypto.clone(),
    );
    let book = books.find_shared_by_name(PRESET_BOOK).await.unwrap();
    let peers = books
        .peers(Actor::Admin, &book.guid, PageRequest::default())
        .await
        .unwrap()
        .items;
    assert_eq!(peers.len(), 1);
    assert_eq!(
        (peers[0].id.as_str(), peers[0].alias.as_str()),
        (DEVICE_ID, "Reception")
    );
    assert_eq!(peers[0].tags, ["printers"]);
}

/// The client uploads again after every restart and username change: the same content is
/// answered SYSINFO_UPDATED without touching the row.
#[tokio::test]
async fn an_unchanged_upload_is_acknowledged_without_a_write() {
    // Arrange
    let app = app_with(&["registered-device"]).await;
    let first = app.send(&sysinfo_request("pc", json!({}))).await.unwrap();
    let repo = DeviceRepository::new(app.state.db.clone());
    let before = repo.find_by_rustdesk_id(DEVICE_ID).await.unwrap().unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // Act
    let second = app.send(&sysinfo_request("pc", json!({}))).await.unwrap();

    // Assert
    assert_eq!(text(&first), "SYSINFO_UPDATED");
    assert_eq!((second.status, text(&second)), (200, "SYSINFO_UPDATED"));
    let after = repo.find_by_rustdesk_id(DEVICE_ID).await.unwrap().unwrap();
    assert_eq!(after, before);
}

/// With `devices.verify_uuid` off, a wrong uuid is accepted for a registered id; an unknown
/// id still is not.
#[tokio::test]
async fn verify_uuid_off_accepts_any_uuid_of_a_registered_id() {
    // Arrange
    let app = app_with(&["registered-device"]).await;
    app.set_setting("devices.verify_uuid", json!(false))
        .await
        .unwrap();
    let wrong_uuid = sysinfo_request("pc", json!({ "uuid": "b3RoZXI=" }));
    let unknown = sysinfo_request("pc", json!({ "id": "555555555" }));

    // Act
    let accepted = app.send(&wrong_uuid).await.unwrap();
    let refused = app.send(&unknown).await.unwrap();

    // Assert
    assert_eq!(text(&accepted), "SYSINFO_UPDATED");
    assert_eq!(text(&refused), "ID_NOT_FOUND");
}

/// The route-level body limit answers 413 before anything is parsed.
#[tokio::test]
async fn an_oversized_body_is_refused() {
    // Arrange
    let app = app_with(&["registered-device"]).await;
    let filler = "x".repeat(deskhaven_client_api::MAX_SYSINFO_BODY);
    let request = sysinfo_request("pc", json!({ "padding": filler }));

    // Act
    let reply = app.send(&request).await.unwrap();

    // Assert
    assert_eq!(reply.status, 413);
}

/// A second changing upload within 30 s is answered 429 (not stored): the client's next try
/// is 120 s later (`sync.rs:221-228`), past the window, so it never loops. Unchanged
/// uploads stay `SYSINFO_UPDATED`.
#[tokio::test]
async fn a_changing_upload_right_after_another_is_answered_429() {
    // Arrange
    let app = app_with(&["registered-device"]).await;
    app.send(&sysinfo_request("one", json!({}))).await.unwrap();

    // Act
    let changed = app.send(&sysinfo_request("two", json!({}))).await.unwrap();
    let same = app.send(&sysinfo_request("one", json!({}))).await.unwrap();

    // Assert
    assert_eq!(changed.status, 429);
    assert_eq!(
        changed.json::<ErrorBody>().unwrap().error,
        TRY_LATER_MESSAGE
    );
    assert!(changed.headers.contains_key("retry-after"));
    assert_eq!((same.status, text(&same)), (200, "SYSINFO_UPDATED"));
}

/// Requests from one client network beyond the sysinfo quota are answered 429; another
/// network is unaffected.
#[tokio::test]
async fn requests_beyond_the_per_ip_quota_are_answered_429() {
    // Arrange
    let app = app_with(&["registered-device"]).await;
    let request = sysinfo_request("pc", json!({}));
    for _ in 0..SYSINFO_PER_IP_PER_MINUTE {
        app.send(&request).await.unwrap();
    }

    // Act
    let over = app.send(&request).await.unwrap();
    let elsewhere = app
        .send(&from(request.clone(), "198.51.100.9:40000"))
        .await
        .unwrap();

    // Assert
    assert_eq!(over.status, 429);
    assert_eq!(text(&elsewhere), "SYSINFO_UPDATED");
}
