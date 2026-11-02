//! Legacy `/api/ab` flows (03 §5.2): what one API writes the other reads, unchanged; refused
//! pushes leave the book as it was; gzip is negotiated, not assumed. Documents are decoded
//! with the client mirror types (`LegacyAbReply`), the new API's replies as raw JSON so
//! every field is compared.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_client_api::MAX_LEGACY_AB_BODY;
use deskhaven_contract::ab_seeds::PERSONAL_PEER;
use deskhaven_contract::app::TestApp;
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::{ErrorBody, LegacyAbData, LegacyAbReply};
use deskhaven_contract::seeds::{SeedOutputs, Seeds};
use deskhaven_db::entity::{address_book_peer, address_book_tag};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

const FULL_MESSAGE: &str = "You have reached the maximum number of managed devices.";

fn request(method: &str, path: &str, token: &str, body: Option<Value>) -> Request {
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

fn text(reply: &Reply) -> String {
    String::from_utf8_lossy(&reply.body).into_owned()
}

/// `GET /api/ab` as the Flutter client sends it.
async fn pull_reply(app: &TestApp, token: &str) -> Reply {
    let mut get = request("GET", "/api/ab", token, None);
    get.headers
        .insert("accept-encoding".to_owned(), "gzip".to_owned());
    send(app, get).await
}

/// The document a legacy client holds after a pull; `None` for `null`.
async fn pull(app: &TestApp, token: &str) -> Option<LegacyAbData> {
    let reply = pull_reply(app, token).await;
    assert_eq!(reply.status, 200, "{}", text(&reply));
    let parsed: Option<LegacyAbReply> = reply.json().unwrap();
    parsed.map(|reply| reply.data)
}

/// The raw `data` string of a pull (the exact bytes a client re-parses).
async fn pull_data(app: &TestApp, token: &str) -> Option<String> {
    let reply = pull_reply(app, token).await;
    let value: Value = reply.json().unwrap();
    value
        .get("data")
        .map(|data| data.as_str().unwrap().to_owned())
}

/// `POST /api/ab` with `document` encoded the way `pushAb` does (a JSON string in `data`).
async fn push_document(app: &TestApp, token: &str, document: &Value) -> Reply {
    push_raw(app, token, &document.to_string()).await
}

async fn push_raw(app: &TestApp, token: &str, data: &str) -> Reply {
    let body = json!({ "data": data });
    send(app, request("POST", "/api/ab", token, Some(body))).await
}

/// A document as `_serialize()` writes it: `tag_colors` is itself a JSON string.
fn document(tags: &[(&str, i64)], peers: Vec<Value>) -> Value {
    let names: Vec<_> = tags.iter().map(|(name, _)| *name).collect();
    let colors: serde_json::Map<String, Value> = tags
        .iter()
        .map(|(name, color)| ((*name).to_owned(), json!(color)))
        .collect();
    json!({ "tags": names, "peers": peers, "tag_colors": Value::Object(colors).to_string() })
}

/// A peer as `toCustomJson(includingHash: true)` writes it.
fn legacy_peer(id: &str, alias: &str, tags: &[&str], hash: &str) -> Value {
    json!({
        "id": id, "username": "u", "hostname": "h", "platform": "Linux",
        "alias": alias, "tags": tags, "hash": hash
    })
}

fn assert_done(reply: &Reply) {
    assert_eq!((reply.status, text(reply).as_str()), (200, ""));
}

fn assert_refused(reply: &Reply, status: u16) -> String {
    assert_eq!(reply.status, status, "{}", text(reply));
    reply.json::<ErrorBody>().unwrap().error
}

/// The new API's view of the book: `(peers as the client reads them, tags)`.
async fn new_api(app: &TestApp, token: &str, guid: &str) -> (Vec<Value>, Vec<Value>) {
    let path = format!("/api/ab/peers?current=1&pageSize=100&ab={guid}");
    let peers = send(app, request("POST", &path, token, None)).await;
    let tags = send(
        app,
        request("POST", &format!("/api/ab/tags/{guid}"), token, None),
    )
    .await;
    let page: Value = peers.json().unwrap();
    (
        page["data"].as_array().unwrap().clone(),
        tags.json::<Value>().unwrap().as_array().unwrap().clone(),
    )
}

#[tokio::test]
async fn a_legacy_push_is_what_the_new_api_reads() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let doc = document(
        &[("work", 0xFF00_00FF), ("ops", 0xFF00_FF00)],
        vec![
            legacy_peer(PERSONAL_PEER, "desk", &["ops"], "new-hash"),
            legacy_peer("555555555", "", &[], ""),
        ],
    );

    let pushed = push_document(&app, token, &doc).await;
    let (peers, tags) = new_api(&app, token, guid).await;

    assert_done(&pushed);
    let ids: Vec<_> = peers.iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, [PERSONAL_PEER, "555555555"]);
    let first = &peers[0];
    for (key, want) in [
        ("alias", "desk"),
        ("username", "u"),
        ("hostname", "h"),
        ("platform", "Linux"),
        ("hash", "new-hash"),
    ] {
        assert_eq!(first[key], want, "{key}");
    }
    assert_eq!(first["tags"], json!(["ops"]));
    assert!(first.get("password").is_none(), "{first}");
    assert_eq!(
        tags,
        [
            json!({ "name": "ops", "color": 0xFF00_FF00_i64 }),
            json!({ "name": "work", "color": 0xFF00_00FF_i64 }),
        ]
    );
}

#[tokio::test]
async fn a_legacy_push_keeps_what_only_the_new_api_knows() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let update =
        json!({ "id": PERSONAL_PEER, "note": "kept", "rdpPort": "3390", "loginName": "l" });
    let path = format!("/api/ab/peer/update/{guid}");
    assert_done(&send(&app, request("PUT", &path, token, Some(update))).await);
    // The seed peer has forceAlwaysRelay "true" and an unknown field.
    let (before, _) = new_api(&app, token, guid).await;

    let doc = document(
        &[("work", 0xFF00_00FF)],
        vec![legacy_peer(PERSONAL_PEER, "office", &["work"], "seed-hash")],
    );
    assert_done(&push_document(&app, token, &doc).await);
    let (after, _) = new_api(&app, token, guid).await;

    assert_eq!(after.len(), 1);
    for key in [
        "note",
        "rdpPort",
        "loginName",
        "forceAlwaysRelay",
        "futureField",
    ] {
        assert_eq!(after[0][key], before[0][key], "{key}");
    }
    assert_eq!(after[0]["forceAlwaysRelay"], "true");
    assert_eq!(after[0]["futureField"], json!({ "kept": true }));
}

#[tokio::test]
async fn a_new_api_write_is_what_a_legacy_pull_reads() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let calls = [
        request(
            "POST",
            &format!("/api/ab/peer/add/{guid}"),
            token,
            Some(json!({
                "id": "444444444", "hash": "h-444", "password": "never", "alias": "lab",
                "username": "root", "hostname": "lab-1", "platform": "Windows",
                "tags": ["work", "new"], "note": "n", "rdpPort": "3390", "extra": 1
            })),
        ),
        request(
            "POST",
            &format!("/api/ab/tag/add/{guid}"),
            token,
            Some(json!({ "name": "new", "color": 0xFF12_3456_i64 })),
        ),
        request(
            "PUT",
            &format!("/api/ab/tag/update/{guid}"),
            token,
            Some(json!({ "name": "work", "color": 0xFFAB_CDEF_i64 })),
        ),
    ];
    for call in calls {
        assert_done(&send(&app, call).await);
    }

    let doc = pull(&app, token).await.unwrap();

    assert_eq!(doc.tags, ["new", "work"]);
    assert_eq!(doc.tag_colors["new"], 0xFF12_3456);
    assert_eq!(doc.tag_colors["work"], 0xFFAB_CDEF);
    let ids: Vec<_> = doc.peers.iter().map(|p| p.id.as_deref().unwrap()).collect();
    assert_eq!(ids, [PERSONAL_PEER, "444444444"]);
    let lab = &doc.peers[1];
    assert_eq!(lab.hash.as_deref(), Some("h-444"));
    assert_eq!(lab.password, None);
    assert_eq!(lab.alias.as_deref(), Some("lab"));
    assert_eq!(lab.username.as_deref(), Some("root"));
    assert_eq!(lab.hostname.as_deref(), Some("lab-1"));
    assert_eq!(lab.platform.as_deref(), Some("Windows"));
    assert_eq!(lab.tags, Some(vec![json!("work"), json!("new")]));
}

#[tokio::test]
async fn pulling_and_pushing_the_document_back_changes_nothing() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let extra = json!({ "id": "444444444", "hash": "h", "note": "n", "rdpPort": "1", "tags": ["x"],
                        "forceAlwaysRelay": "true", "newClientField": [1, { "a": null }] });
    let path = format!("/api/ab/peer/add/{guid}");
    assert_done(&send(&app, request("POST", &path, token, Some(extra))).await);
    let tag = json!({ "name": "x", "color": 0xFF01_0203_i64 });
    let path = format!("/api/ab/tag/add/{guid}");
    assert_done(&send(&app, request("POST", &path, token, Some(tag))).await);
    let before = new_api(&app, token, guid).await;
    let data = pull_data(&app, token).await.unwrap();

    // Sciter re-posts the whole parsed document untouched (`JSON.stringify(ab)`).
    let pushed = push_raw(&app, token, &data).await;
    let after = new_api(&app, token, guid).await;

    assert_done(&pushed);
    assert_eq!(before, after);
    assert_eq!(pull_data(&app, token).await.unwrap(), data);
}

#[tokio::test]
async fn gzip_is_used_when_the_client_accepts_it_and_not_otherwise() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let token = &out["session_token"];
    let with = |value: Option<&str>| {
        let mut get = request("GET", "/api/ab", token, None);
        if let Some(value) = value {
            get.headers
                .insert("accept-encoding".to_owned(), value.to_owned());
        }
        get
    };
    let plain = send(&app, with(None)).await;

    for accepted in [
        "gzip",
        "GZip",
        "deflate, gzip;q=0.5, br",
        "x-gzip",
        "identity, gzip",
    ] {
        let reply = send(&app, with(Some(accepted))).await;
        let encoding = reply.headers.get("content-encoding");
        assert_eq!(
            encoding.map(|v| v.to_str().unwrap()),
            Some("gzip"),
            "{accepted}"
        );
        assert_eq!(reply.body, plain.body, "{accepted}: same document");
    }
    for refused in ["identity", "gzip;q=0", "deflate, br", "gzipped", ""] {
        let reply = send(&app, with(Some(refused))).await;
        assert!(
            reply.headers.get("content-encoding").is_none(),
            "{refused:?}"
        );
        assert_eq!(reply.body, plain.body);
    }
    assert!(plain.headers.get("content-encoding").is_none());
    assert_eq!(plain.headers["vary"], "Accept-Encoding");
}

#[tokio::test]
async fn an_empty_book_reads_null_and_a_push_of_nothing_empties_it() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let token = &out["session_token"];
    assert!(pull(&app, token).await.is_some());

    let emptied = push_document(&app, token, &document(&[], vec![])).await;
    let reply = pull_reply(&app, token).await;

    assert_done(&emptied);
    assert_eq!(text(&reply), "null");
    assert!(pull(&app, token).await.is_none());
    // The Sciter UI starts from {tags: [], peers: []} (no tag_colors).
    let sciter = push_raw(&app, token, r#"{"tags":[],"peers":[{"id":"7"}]}"#).await;
    assert_done(&sciter);
    assert_eq!(pull(&app, token).await.unwrap().peers.len(), 1);
}

#[tokio::test]
async fn the_sciter_pull_returns_the_same_document() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let token = &out["session_token"];

    let reply = send(&app, request("POST", "/api/ab/get", token, Some(json!({})))).await;

    assert_eq!(reply.status, 200, "{}", text(&reply));
    let sciter: Value = reply.json().unwrap();
    let flutter = pull_data(&app, token).await.unwrap();
    assert_eq!(sciter["data"], json!(flutter));
    assert!(sciter["updated_at"].as_i64().unwrap() > 0);
}

#[tokio::test]
async fn refused_pushes_leave_the_book_as_it_was() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let before = (
        new_api(&app, token, guid).await,
        pull_data(&app, token).await,
    );
    let too_long = "x".repeat(65);
    let cases: Vec<(String, &str)> = vec![
        ("{\"tags\": [".to_owned(), "Invalid data"),
        ("null".to_owned(), "Invalid data"),
        ("[]".to_owned(), "Invalid data"),
        (String::new(), "Invalid data"),
        (
            json!({ "tags": "a", "peers": [] }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": [1], "peers": [] }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": [], "peers": {} }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": [], "peers": [], "tag_colors": 5 }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": [], "peers": [], "tag_colors": "[]" }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": ["a"], "peers": [], "tag_colors": "{\"a\":\"red\"}" }).to_string(),
            "Invalid data",
        ),
        (
            json!({ "tags": ["a"], "peers": [], "tag_colors": "{\"a\":1.5}" }).to_string(),
            "Invalid data",
        ),
        (
            document(&[("a", 0x1_0000_0000)], vec![legacy_peer("1", "", &[], "")]).to_string(),
            "Invalid color: must be a 32-bit ARGB value",
        ),
        (
            document(&[("a", -1)], vec![]).to_string(),
            "Invalid color: must be a 32-bit ARGB value",
        ),
        (
            document(&[(too_long.as_str(), 1)], vec![]).to_string(),
            "Invalid name: must be at most 64 characters",
        ),
        (
            json!({ "tags": [""], "peers": [], "tag_colors": "{}" }).to_string(),
            "Invalid name: must not be empty",
        ),
        (
            document(
                &[],
                vec![legacy_peer("1", "", &[], ""), json!({ "alias": "no id" })],
            )
            .to_string(),
            "Invalid id: is required",
        ),
        (
            document(&[], vec![legacy_peer("has space", "", &[], "")]).to_string(),
            "Invalid id: must be 1-128 characters without spaces",
        ),
        (
            document(&[], vec![json!({ "id": "1", "tags": [""] })]).to_string(),
            "Invalid tags: must not be empty",
        ),
    ];

    for (data, error) in cases {
        let reply = push_raw(&app, token, &data).await;
        assert_eq!(assert_refused(&reply, 400), error, "{data}");
    }

    let after = (
        new_api(&app, token, guid).await,
        pull_data(&app, token).await,
    );
    assert_eq!(before, after);
}

#[tokio::test]
async fn the_peer_cap_counts_the_peers_as_sent_and_keeps_the_book() {
    let (app, out) = app_with(&["client-session", "ab-personal", "ab-one-peer-max"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let before = new_api(&app, token, guid).await;
    let two = document(
        &[],
        vec![legacy_peer("1", "", &[], ""), legacy_peer("2", "", &[], "")],
    );
    // Repeated ids still count: the read stops at the cap, it does not deduplicate first.
    let repeated = document(
        &[],
        vec![
            legacy_peer("9", "a", &[], ""),
            legacy_peer("9", "b", &[], ""),
        ],
    );
    let one = document(&[], vec![legacy_peer("9", "only", &[], "")]);

    for refused in [two, repeated] {
        let reply = push_document(&app, token, &refused).await;
        assert_eq!(assert_refused(&reply, 400), FULL_MESSAGE);
    }
    let unchanged = new_api(&app, token, guid).await;
    let accepted = push_document(&app, token, &one).await;

    assert_eq!(unchanged, before);
    assert_done(&accepted);
    let (peers, _) = new_api(&app, token, guid).await;
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["id"], "9");
}

#[tokio::test]
async fn a_push_with_a_huge_peer_list_is_refused_at_the_cap() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let before = new_api(&app, token, guid).await;
    let list: Vec<String> = (0..50_000).map(|i| format!(r#"{{"id":"{i}"}}"#)).collect();
    let data = format!(r#"{{"tags":[],"peers":[{}]}}"#, list.join(","));
    assert!(data.len() < MAX_LEGACY_AB_BODY, "{}", data.len());

    let reply = push_raw(&app, token, &data).await;

    assert_eq!(assert_refused(&reply, 400), FULL_MESSAGE);
    assert_eq!(new_api(&app, token, guid).await, before);
}

/// A `data` string for one peer `keep`, padded with an ignored key until the whole request
/// body (`{"data":"..."}`) is exactly `body_len` bytes.
fn padded_data(body_len: usize) -> String {
    let data = |pad: usize| {
        format!(
            r#"{{"pad":"{}","peers":[{{"id":"keep"}}]}}"#,
            "x".repeat(pad)
        )
    };
    let bare = json!({ "data": data(0) }).to_string().len();
    let padded = data(body_len - bare);
    assert_eq!(json!({ "data": padded }).to_string().len(), body_len);
    padded
}

#[tokio::test]
async fn push_bodies_are_limited_by_the_stacks_limit_not_the_per_item_one() {
    use deskhaven_client_api::MAX_AB_BODY;
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let before = new_api(&app, token, guid).await;

    // Above the per-item routes' limit, so the legacy route has a limit of its own.
    let above_item_limit = push_raw(&app, token, &padded_data(MAX_AB_BODY + 1)).await;
    // Exactly at the effective limit: the global one (2 MiB), which the route limit equals.
    let at_limit = push_raw(&app, token, &padded_data(MAX_LEGACY_AB_BODY)).await;
    let (stored, _) = new_api(&app, token, guid).await;
    let over = push_raw(&app, token, &padded_data(MAX_LEGACY_AB_BODY + 1)).await;

    assert_done(&above_item_limit);
    assert_done(&at_limit);
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0]["id"], "keep");
    assert!(!assert_refused(&over, 413).is_empty());
    assert_eq!(
        new_api(&app, token, guid).await.0,
        stored,
        "413 changed nothing"
    );
    assert_ne!(before.0, stored);
}

#[tokio::test]
async fn a_large_book_round_trips_within_the_limit() {
    let (app, out) = app_with(&["client-session"]).await;
    let token = &out["session_token"];
    let peers: Vec<_> = (0..1000)
        .map(|i| {
            legacy_peer(
                &format!("{i:09}"),
                &format!("alias-{i}"),
                &["bulk"],
                &format!("hash-{i:0>40}"),
            )
        })
        .collect();
    let doc = document(&[("bulk", 0xFF00_0001)], peers);
    assert!(doc.to_string().len() < MAX_LEGACY_AB_BODY / 4);

    let pushed = push_document(&app, token, &doc).await;
    let pulled = pull(&app, token).await.unwrap();

    assert_done(&pushed);
    assert_eq!(pulled.peers.len(), 1000);
    assert_eq!(pulled.peers[999].alias.as_deref(), Some("alias-999"));
    assert_eq!(pulled.tags, ["bulk"]);
}

#[tokio::test]
async fn passwords_are_dropped_and_hashes_sealed_at_rest() {
    let (app, out) = app_with(&["client-session"]).await;
    let token = &out["session_token"];
    let mut peer = legacy_peer("1", "", &[], "secret-hash-value");
    peer["password"] = json!("plain-password");
    peer["device_group_name"] = json!("server-derived");

    assert_done(&push_document(&app, token, &document(&[], vec![peer])).await);

    let rows = address_book_peer::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].password_enc.is_none());
    assert!(!rows[0].extra.contains("plain-password") && !rows[0].extra.contains("server-derived"));
    let sealed = rows[0].hash_enc.clone().unwrap();
    assert!(
        sealed.starts_with("v1:") && !sealed.contains("secret-hash-value"),
        "{sealed}"
    );
    let data = pull_data(&app, token).await.unwrap();
    assert!(
        data.contains("secret-hash-value") && !data.contains("plain-password"),
        "{data}"
    );
}

#[tokio::test]
async fn a_push_only_ever_touches_the_senders_personal_book() {
    let (app, out) = app_with(&[
        "client-session",
        "ab-personal",
        "ab-shared-write",
        "ab-foreign",
    ])
    .await;
    let token = &out["session_token"];
    let shared_before = new_api(&app, token, &out["write_guid"]).await;
    let rows_before = address_book_peer::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap()
        .len();

    let doc = document(&[], vec![legacy_peer("777", "mine", &[], "")]);
    assert_done(&push_document(&app, token, &doc).await);

    assert_eq!(
        new_api(&app, token, &out["write_guid"]).await,
        shared_before
    );
    let rows = address_book_peer::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    // Alice's personal peer was replaced (one row out, one in); the shared book's stays.
    assert_eq!(rows.len(), rows_before);
    let tags = address_book_tag::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap();
    assert!(
        tags.iter().all(|t| t.name == "ops"),
        "only the shared book's tag remains"
    );
}

#[tokio::test]
async fn gzip_encoded_pushes_are_refused_and_nothing_is_stored() {
    let (app, out) = app_with(&["client-session", "ab-personal"]).await;
    let (token, guid) = (&out["session_token"], &out["personal_guid"]);
    let before = new_api(&app, token, guid).await;
    let mut push = request("POST", "/api/ab", token, Some(json!({ "data": "{}" })));
    push.headers
        .insert("content-encoding".to_owned(), "gzip".to_owned());

    let reply = send(&app, push).await;

    assert_eq!(assert_refused(&reply, 400), "Unsupported content encoding");
    assert_eq!(new_api(&app, token, guid).await, before);
    let mut identity = request("POST", "/api/ab", token, Some(json!({ "data": "{}" })));
    identity
        .headers
        .insert("content-encoding".to_owned(), "identity".to_owned());
    assert_done(&send(&app, identity).await);
}
