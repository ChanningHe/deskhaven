//! Settings endpoints: list shape (sources, pins, schemas), secret masking, batch update,
//! optimistic-lock conflicts, cross-key rules, reset to the default and history. Runs on
//! SQLite and PostgreSQL. Permissions and audit rows are in `settings_audit.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, Reply, harness, harness_pinned};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_config::bootstrap::{LoadOptions, load};
use deskhaven_config::settings::PinSet;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const SETTINGS: &str = "/admin/api/v1/settings";
const SECRET: &str = "smtp-pass-0042";

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let owner = h.console("own", Role::Owner).await;
    (h, owner)
}

fn find<'a>(list: &'a Value, key: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == key)
        .unwrap_or_else(|| panic!("no setting {key} in {list}"))
}

async fn patch(h: &Harness, owner: &Console, body: Value) -> Reply {
    h.write(Method::PATCH, SETTINGS, owner, body).await
}

fn change(key: &str, version: i64, value: Value) -> Value {
    json!({ "key": key, "version": version, "value": value })
}

fn reset(key: &str, version: i64) -> Value {
    json!({ "key": key, "version": version, "reset": true })
}

#[tokio::test]
async fn the_list_carries_value_default_source_version_category_and_schema() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let reply = h.get(SETTINGS, &owner).await;

        assert_eq!(reply.status, 200, "{}", reply.body);
        let ttl = find(&reply.body, "auth.session_ttl");
        assert_eq!(ttl["value"], "30d");
        assert_eq!(ttl["default"], "30d");
        assert_eq!(ttl["source"], json!({ "kind": "default" }));
        assert_eq!(
            (ttl["pinned"].as_bool(), ttl["version"].as_i64()),
            (Some(false), Some(0))
        );
        assert_eq!(ttl["category"], "auth");
        assert_eq!(ttl["sensitive"], false);
        assert!(!ttl["description"].as_str().unwrap().is_empty());
        assert_eq!(
            ttl["schema"],
            json!({ "type": "duration", "min": "5m", "max": "365d", "zero_allowed": false })
        );
        let min_length = find(&reply.body, "auth.password.min_length");
        assert_eq!(
            min_length["schema"],
            json!({ "type": "u32", "min": 8, "max": 256, "zero_allowed": false })
        );
        let registration = find(&reply.body, "auth.registration");
        assert_eq!(registration["schema"]["type"], "enum");
        assert_eq!(
            registration["schema"]["variants"],
            json!(["disabled", "invite", "open"])
        );
        assert_eq!(
            find(&reply.body, "devices.verify_uuid")["schema"],
            json!({ "type": "bool" })
        );
        let relays = find(&reply.body, "signal.relay_servers");
        assert_eq!(relays["schema"]["type"], "host_port_list");
        assert_eq!(relays["schema"]["max_items"], 32);
        assert_eq!(relays["value"], json!([]));
        assert_eq!(find(&reply.body, "smtp.from")["schema"]["format"], "email");
        // Every category of the registry is there, in declaration order.
        let categories: Vec<&str> = reply
            .body
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["category"].as_str().unwrap())
            .collect();
        assert_eq!(categories.first(), Some(&"auth"));
        assert!(categories.contains(&"smtp") && categories.contains(&"signal"));
    })
    .await;
}

#[tokio::test]
async fn a_change_is_stored_and_shows_its_author_version_and_time() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let reply = patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.session_ttl", 0, json!("60d"))], "reason": "tune" }),
        )
        .await;

        assert_eq!(reply.status, 200, "{}", reply.body);
        let ttl = find(&reply.body, "auth.session_ttl");
        assert_eq!(ttl["value"], "60d");
        assert_eq!(ttl["version"], 1);
        assert_eq!(ttl["source"]["kind"], "database");
        assert_eq!(ttl["source"]["version"], 1);
        assert_eq!(ttl["source"]["by"], owner.user_id);
        assert_eq!(ttl["source"]["by_username"], "own");
        assert!(ttl["source"]["at"].as_i64().unwrap() > 0);
        assert_eq!(ttl["default"], "30d");
        // The running service sees it at once, and so does the next read.
        let snapshot = h.state.settings.snapshot();
        assert_eq!(
            snapshot
                .get(&deskhaven_config::settings::AUTH_SESSION_TTL)
                .as_std()
                .as_secs(),
            60 * 86_400
        );
        let again = h.get(SETTINGS, &owner).await;
        assert_eq!(find(&again.body, "auth.session_ttl"), ttl);
        // Text forms are accepted like native ones.
        let more = patch(
            &h,
            &owner,
            json!({ "changes": [
                change("auth.password.min_length", 0, json!("12")),
                change("signal.relay_servers", 0, json!(["Relay.example.com:21117"])),
                change("devices.verify_uuid", 0, json!(false)),
            ] }),
        )
        .await;
        assert_eq!(more.status, 200, "{}", more.body);
        assert_eq!(find(&more.body, "auth.password.min_length")["value"], 12);
        assert_eq!(
            find(&more.body, "signal.relay_servers")["value"],
            json!(["relay.example.com:21117"])
        );
    })
    .await;
}

#[tokio::test]
async fn secrets_are_write_only_on_every_path() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let reply = patch(
            &h,
            &owner,
            json!({ "changes": [change("smtp.password", 0, json!(SECRET))] }),
        )
        .await;

        assert_eq!(reply.status, 200, "{}", reply.body);
        let password = find(&reply.body, "smtp.password");
        assert_eq!(password["value"], json!({ "set": true }));
        assert_eq!(password["default"], json!({ "set": false }));
        assert_eq!(password["sensitive"], true);
        assert_eq!(password["schema"]["type"], "secret");
        let listed = h.get(SETTINGS, &owner).await;
        let history = h
            .get(&format!("{SETTINGS}/smtp.password/history"), &owner)
            .await;
        assert_eq!(
            history.body["items"][0]["new_value"],
            json!({ "set": true })
        );
        // A stale secret change: the 409 details do not leak it either.
        let stale = patch(
            &h,
            &owner,
            json!({ "changes": [change("smtp.password", 0, json!("other-secret-9"))] }),
        )
        .await;
        assert_eq!(
            (stale.status.as_u16(), stale.code()),
            (409, "version_conflict")
        );
        assert_eq!(
            stale.body["error"]["details"]["conflicts"][0]["value"],
            json!({ "set": true })
        );
        for body in [&reply.body, &listed.body, &history.body, &stale.body] {
            let text = body.to_string();
            assert!(
                !text.contains(SECRET) && !text.contains("other-secret-9"),
                "{text}"
            );
        }
        // Resetting it unsets it; the history keeps only the marker.
        let cleared = patch(
            &h,
            &owner,
            json!({ "changes": [reset("smtp.password", 1)] }),
        )
        .await;
        assert_eq!(
            find(&cleared.body, "smtp.password")["value"],
            json!({ "set": false })
        );
    })
    .await;
}

#[tokio::test]
async fn a_stale_version_is_a_409_with_the_current_values_and_the_batch_is_not_applied() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let other = h.console("own2", Role::Owner).await;
        let first = patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.session_ttl", 0, json!("60d"))] }),
        )
        .await;
        assert_eq!(first.status, 200);

        // The second owner still holds version 0 of the key and edits another key too.
        let conflict = patch(
            &h,
            &other,
            json!({ "changes": [
                change("smtp.port", 0, json!(2525)),
                change("auth.session_ttl", 0, json!("90d")),
            ] }),
        )
        .await;

        assert_eq!(
            (conflict.status.as_u16(), conflict.code()),
            (409, "version_conflict")
        );
        let details = &conflict.body["error"]["details"];
        assert_eq!(details["keys"], json!(["auth.session_ttl"]));
        let current = &details["conflicts"][0];
        assert_eq!(current["key"], "auth.session_ttl");
        assert_eq!(current["value"], "60d");
        assert_eq!(current["version"], 1);
        assert_eq!(current["source"]["by_username"], "own");
        let after = h.get(SETTINGS, &other).await;
        assert_eq!(
            find(&after.body, "smtp.port")["value"],
            587,
            "batch is all or nothing"
        );
        assert_eq!(find(&after.body, "auth.session_ttl")["value"], "60d");
        // Retrying with the version just read goes through.
        let retry = patch(
            &h,
            &other,
            json!({ "changes": [change("auth.session_ttl", 1, json!("90d"))] }),
        )
        .await;
        assert_eq!(find(&retry.body, "auth.session_ttl")["version"], 2);
    })
    .await;
}

#[tokio::test]
async fn invalid_values_unknown_keys_and_malformed_changes_are_422_with_field_details() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let cases = [
            (
                change("auth.session_ttl", 0, json!("1s")),
                "auth.session_ttl",
            ),
            (
                change("auth.password.min_length", 0, json!(3)),
                "auth.password.min_length",
            ),
            (
                change("auth.registration", 0, json!("everyone")),
                "auth.registration",
            ),
            (change("nope.nothing", 0, json!(1)), "nope.nothing"),
            (
                change("signal.relay_servers", 0, json!(["bad"])),
                "signal.relay_servers",
            ),
        ];
        for (body, field) in cases {
            let reply = patch(&h, &owner, json!({ "changes": [body.clone()] })).await;
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (422, "validation_failed"),
                "{body}"
            );
            assert_eq!(
                reply.body["error"]["details"]["fields"][field]
                    .as_array()
                    .map(Vec::len),
                Some(1),
                "{body}: {}",
                reply.body
            );
        }
        let both = json!({ "key": "smtp.port", "version": 0, "value": 25, "reset": true });
        let neither = json!({ "key": "smtp.port", "version": 0 });
        for body in [both, neither] {
            let reply = patch(&h, &owner, json!({ "changes": [body] })).await;
            assert_eq!(reply.status, 422, "{}", reply.body);
            assert!(reply.body["error"]["details"]["fields"]["changes[0]"].is_array());
        }
        for body in [
            json!({ "changes": [] }),
            json!({ "changes": [change("smtp.port", -1, json!(25))] }),
            json!({ "changes": [{ "key": "smtp.port", "version": 0, "value": 25, "extra": 1 }] }),
            json!({ "changes": [change("smtp.port", 0, json!(25))], "reason": "x".repeat(501) }),
        ] {
            assert_eq!(patch(&h, &owner, body).await.status, 422);
        }
        // One bad value rejects the whole batch.
        let mixed = patch(
            &h,
            &owner,
            json!({ "changes": [
                change("smtp.port", 0, json!(2525)),
                change("auth.session_ttl", 0, json!("1s")),
            ] }),
        )
        .await;
        assert_eq!(mixed.status, 422);
        let list = h.get(SETTINGS, &owner).await;
        assert_eq!(find(&list.body, "smtp.port")["value"], 587);
    })
    .await;
}

#[tokio::test]
async fn cross_key_rules_are_422_on_the_candidate_state_of_the_whole_batch() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let refused = patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.email_code.enabled", 0, json!(true))] }),
        )
        .await;
        assert_eq!(
            (refused.status.as_u16(), refused.code()),
            (422, "validation_failed")
        );
        let fields = &refused.body["error"]["details"]["fields"];
        assert_eq!(
            fields["auth.email_code.enabled"].as_array().map(Vec::len),
            Some(1)
        );

        let together = patch(
            &h,
            &owner,
            json!({ "changes": [
                change("auth.email_code.enabled", 0, json!(true)),
                change("smtp.host", 0, json!("mail.example.com")),
                change("smtp.from", 0, json!("noreply@example.com")),
            ] }),
        )
        .await;
        assert_eq!(together.status, 200, "{}", together.body);

        // Resetting a prerequisite breaks the rule too, and changes nothing.
        let broken = patch(&h, &owner, json!({ "changes": [reset("smtp.host", 1)] })).await;
        assert_eq!(
            (broken.status.as_u16(), broken.code()),
            (422, "validation_failed")
        );
        assert!(broken.body["error"]["details"]["fields"]["auth.email_code.enabled"].is_array());
        let list = h.get(SETTINGS, &owner).await;
        assert_eq!(find(&list.body, "smtp.host")["value"], "mail.example.com");
    })
    .await;
}

/// Pins from a real bootstrap file, so the source is the file's path.
fn pinned_from_file(dir: &tempfile::TempDir) -> (PinSet, String) {
    let path = dir.path().join("deskhaven.toml");
    let content = "[settings]\n\"auth.registration\" = \"invite\"\n\"auth.session_ttl\" = \"7d\"\n";
    std::fs::write(&path, content).unwrap();
    let options = LoadOptions {
        config_path: Some(path.clone()),
        search_paths: Vec::new(),
        cli: Default::default(),
    };
    let loaded = load(&options).unwrap();
    let pins = PinSet::from_bootstrap(&loaded.config.settings).unwrap();
    (pins, path.display().to_string())
}

#[tokio::test]
async fn pinned_settings_show_their_file_and_refuse_changes_and_resets() {
    each_migrated_backend(async |db: &TestDb| {
        let dir = tempfile::tempdir().unwrap();
        let (pins, path) = pinned_from_file(&dir);
        let h = harness_pinned(db, AdminRoutes::new(), pins).await;
        let owner = h.console("own", Role::Owner).await;

        let list = h.get(SETTINGS, &owner).await;
        let ttl = find(&list.body, "auth.session_ttl");
        assert_eq!(ttl["pinned"], true);
        assert_eq!(ttl["value"], "7d");
        assert_eq!(ttl["source"], json!({ "kind": "file", "path": path }));
        assert_eq!(find(&list.body, "smtp.port")["pinned"], false);

        for body in [
            json!({ "changes": [
                change("smtp.port", 0, json!(2525)),
                change("auth.session_ttl", 0, json!("1d")),
            ] }),
            json!({ "changes": [reset("auth.registration", 0)] }),
        ] {
            let reply = patch(&h, &owner, body).await;
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (409, "setting_pinned")
            );
            assert!(
                reply.body["error"]["details"]["keys"]
                    .as_array()
                    .unwrap()
                    .len()
                    == 1
            );
        }
        let after = h.get(SETTINGS, &owner).await;
        assert_eq!(find(&after.body, "smtp.port")["value"], 587);
    })
    .await;
}

#[tokio::test]
async fn a_reset_deletes_the_stored_value_and_the_history_says_so() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.session_ttl", 0, json!("60d"))] }),
        )
        .await;

        let reply = patch(
            &h,
            &owner,
            json!({ "changes": [reset("auth.session_ttl", 1)] }),
        )
        .await;

        assert_eq!(reply.status, 200, "{}", reply.body);
        let ttl = find(&reply.body, "auth.session_ttl");
        assert_eq!(ttl["value"], "30d");
        assert_eq!(ttl["source"], json!({ "kind": "default" }));
        assert_eq!(ttl["version"], 0);
        let history = h
            .get(&format!("{SETTINGS}/auth.session_ttl/history"), &owner)
            .await;
        let items = history.body["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["reset"], true);
        assert_eq!(
            (items[0]["old_value"].clone(), items[0]["new_value"].clone()),
            (json!("60d"), json!("30d"))
        );
        assert_eq!(items[0]["version"], 2);
        assert_eq!(items[1]["reset"], false);
        assert_eq!(items[1]["old_value"], Value::Null);

        // The next change continues the version line; the version read before the reset
        // can never match again.
        let again = patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.session_ttl", 0, json!("45d"))] }),
        )
        .await;
        assert_eq!(find(&again.body, "auth.session_ttl")["version"], 3);
        let stale = patch(
            &h,
            &owner,
            json!({ "changes": [change("auth.session_ttl", 1, json!("10d"))] }),
        )
        .await;
        assert_eq!(stale.code(), "version_conflict");
        // Resetting what is not stored changes nothing.
        let none = patch(&h, &owner, json!({ "changes": [reset("smtp.port", 0)] })).await;
        assert_eq!(none.status, 200);
        let port = h
            .get(&format!("{SETTINGS}/smtp.port/history"), &owner)
            .await;
        assert_eq!(port.body["total"], 0);
    })
    .await;
}

#[tokio::test]
async fn history_is_paged_newest_first_with_authors_and_reasons() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let other = h.console("own2", Role::Owner).await;
        for (version, ttl) in ["10d", "20d", "30d"].into_iter().enumerate() {
            let who = if version == 1 { &other } else { &owner };
            let body = json!({
                "changes": [change("auth.session_ttl", i64::try_from(version).unwrap(), json!(ttl))],
                "reason": format!("round {version}"),
            });
            assert_eq!(patch(&h, who, body).await.status, 200);
        }

        let path = format!("{SETTINGS}/auth.session_ttl/history");
        let first = h.get(&format!("{path}?page=1&page_size=2"), &owner).await;
        let second = h.get(&format!("{path}?page=2&page_size=2"), &owner).await;

        assert_eq!(first.status, 200, "{}", first.body);
        assert_eq!(
            (first.body["total"].as_u64(), first.body["page"].as_u64(), first.body["page_size"].as_u64()),
            (Some(3), Some(1), Some(2))
        );
        let items = first.body["items"].as_array().unwrap();
        let shown: Vec<_> = items
            .iter()
            .map(|e| (e["version"].as_i64().unwrap(), e["new_value"].as_str().unwrap()))
            .collect();
        assert_eq!(shown, [(3, "30d"), (2, "20d")]);
        assert_eq!(items[0]["updated_by_username"], "own");
        assert_eq!(items[1]["updated_by_username"], "own2");
        assert_eq!(items[1]["reason"], "round 1");
        assert!(items[0]["updated_at"].as_i64().unwrap() > 0);
        assert_eq!(second.body["items"].as_array().unwrap().len(), 1);
        assert_eq!(second.body["items"][0]["old_value"], Value::Null);

        let unknown = h.get(&format!("{SETTINGS}/nope.nothing/history"), &owner).await;
        assert_eq!((unknown.status.as_u16(), unknown.code()), (404, "not_found"));
        let bad_page = h.get(&format!("{path}?page_size=0"), &owner).await;
        assert_eq!(bad_page.status, 422);
        let empty = h.get(&format!("{SETTINGS}/smtp.port/history"), &owner).await;
        assert_eq!(empty.body["total"], 0);
    })
    .await;
}

#[tokio::test]
async fn a_change_that_leaves_the_value_alone_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;

        let reply = patch(
            &h,
            &owner,
            json!({ "changes": [change("smtp.port", 0, json!(587))] }),
        )
        .await;

        assert_eq!(reply.status, 200);
        assert_eq!(find(&reply.body, "smtp.port")["version"], 0);
        assert_eq!(
            find(&reply.body, "smtp.port")["source"],
            json!({ "kind": "default" })
        );
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}
