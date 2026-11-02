//! User endpoints: list (pagination, filters, sort), detail, create, update, account
//! actions, delete, last-owner and self-delete protection, and their audit rows (before /
//! after, secrets masked). Every test runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_config::REDACTED;
use deskhaven_core::groups::NewGroup;
use deskhaven_core::sessions::{ClientDevice, SessionKind};
use deskhaven_core::users::{Credential, NewUser, Password, Role, Status};
use deskhaven_db::entity::auth_lockout;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait};
use serde_json::{Value, json};

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("root", Role::Owner).await;
    (h, admin)
}

/// A client session token of `username` (password [`PASSWORD`]).
async fn client_token(h: &Harness, username: &str) -> String {
    let auth = h
        .state
        .users
        .authenticate_local(username, &Password::new(PASSWORD))
        .await
        .unwrap();
    let device = ClientDevice {
        rustdesk_id: "123456789".to_owned(),
        uuid: "uuid-1".to_owned(),
        info: None,
    };
    let (_, token) = h
        .state
        .sessions
        .create_client(auth.user.id, &auth.stamp, &device, None)
        .await
        .unwrap();
    token.expose().to_owned()
}

async fn client_session_valid(h: &Harness, token: &str) -> bool {
    let auth = h.state.sessions.authenticate(token, SessionKind::Client);
    auth.await.is_ok()
}

fn json_of(text: Option<&str>) -> Value {
    serde_json::from_str(text.unwrap()).unwrap()
}

fn usernames(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items
        .iter()
        .map(|u| u["username"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn list_paginates_filters_and_sorts() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let alice = h.create_user("alice", Role::Admin).await;
        let bob = h.create_user("bob", Role::User).await;
        h.state
            .users
            .set_status(bob, Status::Disabled)
            .await
            .unwrap();
        let carol = h
            .state
            .users
            .create(NewUser::new("carol", Credential::Ldap));
        let carol = carol.await.unwrap().id;
        let group = h.state.groups.create(NewGroup::new("ops")).await.unwrap();
        h.state.groups.add_member(group.id, carol).await.unwrap();

        let page = h
            .get(
                "/admin/api/v1/users?page=2&page_size=2&sort=-username",
                &root,
            )
            .await;
        assert_eq!(page.status, 200, "{}", page.body);
        assert_eq!(usernames(&page.body), ["bob", "alice"]);
        assert_eq!(
            (page.body["total"].as_u64(), page.body["page"].as_u64()),
            (Some(4), Some(2))
        );

        let in_group = format!("group={}", group.id);
        let cases = [
            ("q=AL", vec!["alice"]),
            ("role=admin", vec!["alice"]),
            ("status=disabled", vec!["bob"]),
            ("auth_source=ldap", vec!["carol"]),
            (in_group.as_str(), vec!["carol"]),
            ("sort=role", vec!["alice", "root", "bob", "carol"]),
        ];
        for (query, expected) in cases {
            let reply = h.get(&format!("/admin/api/v1/users?{query}"), &root).await;
            assert_eq!(usernames(&reply.body), expected, "{query}");
        }
        let item = &h.get("/admin/api/v1/users?q=alice", &root).await.body["items"][0];
        assert_eq!(item["id"], alice);
        assert_eq!(item["auth_source"], "local");
        assert!(item.get("password_hash").is_none());

        let long_q = format!("q={}", "x".repeat(201));
        for bad in [
            "sort=password_hash",
            "page_size=201",
            "role=root",
            long_q.as_str(),
        ] {
            let reply = h.get(&format!("/admin/api/v1/users?{bad}"), &root).await;
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (422, "validation_failed"),
                "{bad}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn detail_shows_groups_and_usable_sessions() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let group = h.state.groups.create(NewGroup::new("ops")).await.unwrap();
        h.state.groups.add_member(group.id, bob).await.unwrap();
        client_token(&h, "bob").await;

        let reply = h.get(&format!("/admin/api/v1/users/{bob}"), &root).await;
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body["user"]["username"], "bob");
        assert_eq!(
            reply.body["groups"],
            json!([{ "id": group.id, "name": "ops" }])
        );
        let session = &reply.body["sessions"][0];
        assert_eq!(
            (session["kind"].as_str(), session["rustdesk_id"].as_str()),
            (Some("client"), Some("123456789"))
        );
        assert!(session.get("token_hash").is_none());

        for missing in ["/admin/api/v1/users/999999", "/admin/api/v1/users/abc"] {
            let reply = h.get(missing, &root).await;
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (404, "not_found"),
                "{missing}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn create_generates_a_temporary_password_shown_once() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let body = json!({ "username": "Dave", "email": "dave@example.com", "role": "admin" });
        let reply = h.write(Method::POST, "/admin/api/v1/users", &root, body).await;
        assert_eq!(reply.status, 201, "{}", reply.body);
        let temporary = reply.body["temporary_password"].as_str().unwrap().to_owned();
        let user = &reply.body["user"];
        assert_eq!((user["role"].as_str(), user["must_change_password"].as_bool()), (Some("admin"), Some(true)));
        let auth = h.state.users.authenticate_local("dave", &Password::new(&temporary)).await;
        assert!(auth.is_ok());

        let rows = h.audit_rows().await;
        let row = rows.last().unwrap();
        assert_eq!((row.action.as_str(), row.target_type.as_str()), ("user.create", "user"));
        assert_eq!(row.target_id, user["id"].to_string());
        assert_eq!(row.actor_id, Some(root.user_id));
        assert_eq!(row.ip.as_deref(), Some("203.0.113.9"));
        assert!(row.before.is_none());
        let after = json_of(row.after.as_deref());
        assert_eq!((after["username"].as_str(), after["role"].as_str()), (Some("Dave"), Some("admin")));
        assert!(!row.after.as_deref().unwrap().contains(&temporary));

        // A chosen password keeps `must_change_password` as asked and is never returned.
        let chosen = json!({ "username": "erin", "password": "Erin-chose-this-1", "must_change_password": false });
        let reply = h.write(Method::POST, "/admin/api/v1/users", &root, chosen).await;
        assert_eq!(reply.status, 201);
        assert!(reply.body.get("temporary_password").is_none());
        assert_eq!(reply.body["user"]["must_change_password"], false);
    })
    .await;
}

#[tokio::test]
async fn create_reports_invalid_and_taken_fields() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let post = async |body: Value| {
            h.write(Method::POST, "/admin/api/v1/users", &root, body)
                .await
        };
        let weak = post(json!({ "username": "x1", "password": "short" })).await;
        assert_eq!(
            (weak.status.as_u16(), weak.code()),
            (422, "validation_failed")
        );
        assert!(weak.body["error"]["details"]["fields"]["password"].is_array());
        assert!(
            !weak.body.to_string().contains("short"),
            "the password is echoed"
        );
        let bad_name = post(json!({ "username": "has space" })).await;
        assert!(bad_name.body["error"]["details"]["fields"]["username"].is_array());
        let unknown = post(json!({ "username": "x2", "is_admin": true })).await;
        assert_eq!(unknown.status, 422);
        let taken = post(json!({ "username": "ROOT" })).await;
        assert_eq!((taken.status.as_u16(), taken.code()), (409, "conflict"));
        assert_eq!(taken.body["error"]["details"]["field"], "username");
        assert_eq!(h.audit_rows().await.len(), 0);
    })
    .await;
}

#[tokio::test]
async fn update_changes_profile_and_role_in_one_audited_row() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let patch = json!({ "display_name": "Bob B", "email": "bob@example.com", "role": "admin", "note": "on call" });
        let path = format!("/admin/api/v1/users/{bob}");
        let reply = h.write(Method::PATCH, &path, &root, patch).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!((reply.body["display_name"].as_str(), reply.body["role"].as_str()), (Some("Bob B"), Some("admin")));

        let clear = h.write(Method::PATCH, &path, &root, json!({ "email": "" })).await;
        assert_eq!(clear.body["email"], Value::Null);

        let rows = h.audit_rows().await;
        assert_eq!(rows.len(), 2);
        let (before, after) = (json_of(rows[0].before.as_deref()), json_of(rows[0].after.as_deref()));
        assert_eq!(rows[0].action, "user.update");
        assert_eq!((before["role"].as_str(), after["role"].as_str()), (Some("user"), Some("admin")));
        assert_eq!((before["display_name"].as_str(), after["display_name"].as_str()), (Some("bob"), Some("Bob B")));
        assert_eq!(after["must_change"], false, "renamed so the redactor keeps it");
    })
    .await;
}

#[tokio::test]
async fn disable_revokes_sessions_and_enable_does_not_restore_them() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let token = client_token(&h, "bob").await;
        let disable = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/disable"),
                &root,
            )
            .await;
        assert_eq!(
            (disable.status.as_u16(), disable.body["status"].as_str()),
            (200, Some("disabled"))
        );
        assert!(!client_session_valid(&h, &token).await);
        let enable = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/enable"),
                &root,
            )
            .await;
        assert_eq!(enable.body["status"], "active");
        assert!(!client_session_valid(&h, &token).await);
        let actions: Vec<_> = h.audit_rows().await.into_iter().map(|r| r.action).collect();
        assert_eq!(actions, ["user.disable", "user.enable"]);
    })
    .await;
}

#[tokio::test]
async fn reset_password_shows_a_temporary_password_and_masks_it_in_the_audit() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let token = client_token(&h, "bob").await;
        let reply = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/reset-password"),
                &root,
            )
            .await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let temporary = reply.body["temporary_password"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            !client_session_valid(&h, &token).await,
            "sessions are revoked"
        );
        let user = h.state.users.get(bob).await.unwrap();
        assert!(user.must_change_password);
        assert!(
            h.state
                .users
                .authenticate_local("bob", &Password::new(&temporary))
                .await
                .is_ok()
        );

        let row = h.audit_rows().await.pop().unwrap();
        assert_eq!(row.action, "user.reset_password");
        let after = json_of(row.after.as_deref());
        assert_eq!(after["password"], REDACTED);
        assert_eq!(after["must_change"], true);
        assert!(!row.after.unwrap().contains(&temporary));

        let ldap = h
            .state
            .users
            .create(NewUser::new("lou", Credential::Ldap))
            .await
            .unwrap();
        let reply = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{}/reset-password", ldap.id),
                &root,
            )
            .await;
        assert_eq!(
            (reply.status.as_u16(), reply.code()),
            (409, "not_local_account")
        );
    })
    .await;
}

#[tokio::test]
async fn reset_2fa_revoke_sessions_and_unlock() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        h.state.tfa.begin_enrolment(bob).await.unwrap();
        let reset = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/reset-2fa"),
                &root,
            )
            .await;
        assert_eq!(reset.status, 204);
        assert!(!h.state.tfa.status(bob).await.unwrap().pending_enrolment);

        let token = client_token(&h, "bob").await;
        client_token(&h, "bob").await;
        let revoke = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/revoke-sessions"),
                &root,
            )
            .await;
        assert_eq!(revoke.body, json!({ "revoked": 2 }));
        assert!(!client_session_valid(&h, &token).await);

        let now = time::now();
        for key in ["user:bob|*", "user:bob|192.0.2.1", "user:bobby|*"] {
            auth_lockout::ActiveModel {
                key: Set(key.to_owned()),
                failures: Set(5),
                window_started_at: Set(now),
                locked_until: Set(Some(now)),
                lock_level: Set(1),
                updated_at: Set(now),
                last_success_at: Set(None),
            }
            .insert(&h.state.db)
            .await
            .unwrap();
        }
        let unlock = h
            .act(
                Method::POST,
                &format!("/admin/api/v1/users/{bob}/unlock"),
                &root,
            )
            .await;
        assert_eq!(unlock.body, json!({ "cleared": 2 }));
        let left = auth_lockout::Entity::find()
            .count(&h.state.db)
            .await
            .unwrap();
        assert_eq!(left, 1, "bobby's row stays");

        let rows = h.audit_rows().await;
        let actions: Vec<_> = rows.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(
            actions,
            ["user.reset_2fa", "user.revoke_sessions", "user.unlock"]
        );
        let unlock_after = json_of(rows[2].after.as_deref());
        assert_eq!(unlock_after["lockout_entries_cleared"], 2);
        assert_eq!(rows[2].target_id, bob.to_string());
    })
    .await;
}

#[tokio::test]
async fn delete_removes_the_account_but_never_your_own() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let reply = h
            .act(Method::DELETE, &format!("/admin/api/v1/users/{bob}"), &root)
            .await;
        assert_eq!(reply.status, 204);
        assert!(h.state.users.get(bob).await.is_err());
        let again = h
            .act(Method::DELETE, &format!("/admin/api/v1/users/{bob}"), &root)
            .await;
        assert_eq!(again.status, 404);

        let own = h
            .act(
                Method::DELETE,
                &format!("/admin/api/v1/users/{}", root.user_id),
                &root,
            )
            .await;
        assert_eq!((own.status.as_u16(), own.code()), (409, "self_delete"));
        let admin = h.console("adm", Role::Admin).await;
        let own = h
            .act(
                Method::DELETE,
                &format!("/admin/api/v1/users/{}", admin.user_id),
                &admin,
            )
            .await;
        assert_eq!(own.code(), "self_delete");

        let rows = h.audit_rows().await;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].action.as_str(), rows[0].after.is_none()),
            ("user.delete", true)
        );
        assert_eq!(json_of(rows[0].before.as_deref())["username"], "bob");
    })
    .await;
}

#[tokio::test]
async fn the_last_owner_cannot_be_demoted_or_disabled() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, root) = setup(db).await;
        let me = format!("/admin/api/v1/users/{}", root.user_id);
        let demote = h
            .write(Method::PATCH, &me, &root, json!({ "role": "admin" }))
            .await;
        assert_eq!((demote.status.as_u16(), demote.code()), (409, "last_owner"));
        let disable = h.act(Method::POST, &format!("{me}/disable"), &root).await;
        assert_eq!(disable.code(), "last_owner");
        assert_eq!(
            h.state.users.get(root.user_id).await.unwrap().role,
            Role::Owner
        );
        assert!(
            h.audit_rows().await.is_empty(),
            "refused changes leave no audit row"
        );

        // With a second owner, the first may step down.
        let second = h.console("second", Role::Owner).await;
        let demote = h
            .write(Method::PATCH, &me, &second, json!({ "role": "admin" }))
            .await;
        assert_eq!(demote.status, 200);
    })
    .await;
}

#[tokio::test]
async fn nobody_resets_their_own_2fa_through_the_admin_route() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = setup(db).await;
        let own = format!("/admin/api/v1/users/{}/reset-2fa", owner.user_id);
        let reply = h.act(Method::POST, &own, &owner).await;
        assert_eq!((reply.status.as_u16(), reply.code()), (409, "self_service"));
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn reset_password_applies_the_owner_rule_before_looking_at_the_source() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, _owner) = setup(db).await;
        let admin = h.console("adm", Role::Admin).await;
        let ldap_owner = NewUser {
            role: Role::Owner,
            ..NewUser::new("dir-owner", Credential::Ldap)
        };
        let target = h.state.users.create(ldap_owner).await.unwrap();
        let path = format!("/admin/api/v1/users/{}/reset-password", target.id);

        // An admin is refused as for any owner: no hint that it has no local password.
        let reply = h.act(Method::POST, &path, &admin).await;
        assert_eq!(
            (reply.status.as_u16(), reply.code()),
            (403, "owner_required")
        );
        // An owner gets the real answer.
        let owner = h.console("own2", Role::Owner).await;
        let reply = h.act(Method::POST, &path, &owner).await;
        assert_eq!(
            (reply.status.as_u16(), reply.code()),
            (409, "not_local_account")
        );
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}
