//! Linking an OIDC identity to one's own account through the console (M4-09), against the
//! mock IdP: re-authentication first (password, and a code when TOTP is on), a link flow
//! bound to the browser and the account, the confirmation preview, completion, the
//! identity on the account page, self-service unlinking with the password, and an owner
//! with TOTP signing in through the linked identity without it. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;
#[path = "../../deskhaven-core/tests/common/oidc_idp.rs"]
mod oidc_idp;

use axum::http::Method;
use common::login::{cookie_value, harness_with};
use common::own::{ACCOUNT, external_console};
use common::{Console, Harness, PASSWORD};
use deskhaven_admin_api::{AdminRoutes, OIDC_BINDING_COOKIE};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::oidc::NewProvider;
use deskhaven_core::state::StateConfig;
use deskhaven_core::users::{AuthSource, Credential, NewUser, Role};
use deskhaven_db::entity::{console_audit, user_identity};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use oidc_idp::{AUTH_CODE, AuthRequest, CLIENT_ID, CLIENT_SECRET, MockIdp, TokenSpec};
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::{Value, json};

const LINK: &str = "/admin/api/v1/me/identities/link";
const VERIFY: &str = "/admin/api/v1/me/identities/link/verify";
const COMPLETE: &str = "/admin/api/v1/me/identities/link/complete";
const LOGIN_START: &str = "/admin/api/v1/auth/oidc/start";
const LOGIN_COMPLETE: &str = "/admin/api/v1/auth/oidc/complete";

struct Env {
    h: Harness,
    idp: MockIdp,
}

async fn env(db: &TestDb) -> Env {
    let config = StateConfig {
        public_url: "https://rd.example.com".to_owned(),
        ..StateConfig::default()
    };
    let h = harness_with(db, AdminRoutes::new(), &config).await;
    let idp = MockIdp::start().await;
    let mut new = NewProvider::new("corp", idp.issuer.clone(), CLIENT_ID);
    new.client_secret = Some(CLIENT_SECRET.to_owned());
    new.display_name = "Corp SSO".to_owned();
    h.state.oidc.create_provider(new).await.unwrap();
    Env { h, idp }
}

/// `console` with the link flow's binding cookie next to its session cookie.
fn with_binding(console: &Console, binding: &str) -> Console {
    Console {
        user_id: console.user_id,
        cookie: format!("{}; {OIDC_BINDING_COOKIE}={binding}", console.cookie),
        csrf: console.csrf.clone(),
    }
}

impl Env {
    async fn start(&self, console: &Console, proof: Value) -> common::Reply {
        let mut body = json!({ "provider": "corp" });
        body.as_object_mut()
            .unwrap()
            .extend(proof.as_object().unwrap().clone());
        self.h.post(LINK, Some(console), body).await
    }

    /// A started link flow: the IdP request and the binding cookie value.
    async fn started(&self, console: &Console, proof: Value) -> (AuthRequest, String) {
        let reply = self.start(console, proof).await;
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        let url = url::Url::parse(reply.body["url"].as_str().unwrap()).unwrap();
        let binding = cookie_value(&reply, OIDC_BINDING_COOKIE).unwrap();
        (AuthRequest::parse(&url), binding)
    }

    async fn mount(&self, subject: &str, request: &AuthRequest) {
        let spec = TokenSpec::for_flow(subject, &request.nonce, SystemClock.now_ms());
        self.idp.mount_token(&spec).await;
    }

    /// The confirmation page's first call: the IdP's answer, exchanged once.
    async fn verify(&self, console: &Console, request: &AuthRequest) -> common::Reply {
        let body = json!({ "state": request.state, "code": AUTH_CODE });
        self.h.post(VERIFY, Some(console), body).await
    }

    async fn complete(&self, console: &Console, request: &AuthRequest) -> common::Reply {
        let body = json!({ "state": request.state });
        self.h.post(COMPLETE, Some(console), body).await
    }

    /// Start, IdP answer for `subject`, verification and confirmation in the same browser.
    async fn link(&self, console: &Console, subject: &str, proof: Value) -> common::Reply {
        let (request, binding) = self.started(console, proof).await;
        self.mount(subject, &request).await;
        let browser = with_binding(console, &binding);
        let verified = self.verify(&browser, &request).await;
        if verified.status != 200 {
            return verified;
        }
        self.complete(&browser, &request).await
    }
}

fn password() -> Value {
    json!({ "current_password": PASSWORD })
}

fn reason(reply: &common::Reply) -> &str {
    reply.body["error"]["details"]["reason"]
        .as_str()
        .unwrap_or("")
}

async fn audit_actions(db: &TestDb) -> Vec<String> {
    console_audit::Entity::find()
        .all(&db.conn)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.action)
        .collect()
}

#[tokio::test]
async fn linking_needs_the_password_and_the_code_when_totp_is_on() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.h.console("alice", Role::User).await;
        let tina = env.h.console("tina", Role::User).await;
        let totp = env.h.enrol(tina.user_id).await;
        let b = db.backend;

        let missing = env.start(&alice, json!({})).await;
        let wrong = env
            .start(&alice, json!({ "current_password": "nope-nope" }))
            .await;
        let no_code = env.start(&tina, password()).await;
        let bad_code = env
            .start(
                &tina,
                json!({ "current_password": PASSWORD, "code": "000000" }),
            )
            .await;
        let with_code = env
            .start(
                &tina,
                json!({ "current_password": PASSWORD, "code": totp.next_code() }),
            )
            .await;
        let fine = env.start(&alice, password()).await;

        assert_eq!(missing.status, 422, "{b}: {:?}", missing.body);
        assert_eq!(wrong.status, 422, "{b}");
        assert_eq!(wrong.body["error"]["code"], "wrong_password", "{b}");
        assert_eq!(no_code.status, 422, "{b}: {:?}", no_code.body);
        assert_eq!(bad_code.body["error"]["code"], "invalid_code", "{b}");
        assert_eq!(with_code.status, 200, "{b}: {:?}", with_code.body);
        assert_eq!(fine.status, 200, "{b}: {:?}", fine.body);
        assert!(cookie_value(&fine, OIDC_BINDING_COOKIE).is_some(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_flow_completes_only_in_the_starting_browser_for_the_starting_account() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.h.console("alice", Role::User).await;
        let bob = env.h.console("bob", Role::User).await;
        let (request, binding) = env.started(&alice, password()).await;
        env.mount("sub-a", &request).await;
        let b = db.backend;

        let browser = with_binding(&alice, &binding);
        let unverified = env.complete(&browser, &request).await;
        let no_cookie = env.verify(&alice, &request).await;
        let other_account = env.verify(&with_binding(&bob, &binding), &request).await;
        let preview = env.verify(&browser, &request).await;
        let reload = env.verify(&browser, &request).await;
        let bob_confirms = env.complete(&with_binding(&bob, &binding), &request).await;
        let done = env.complete(&browser, &request).await;
        let account = env.h.as_console(Method::GET, ACCOUNT, &alice, None).await;

        assert_eq!(reason(&unverified), "expired", "{b}: not verified yet");
        assert_eq!(no_cookie.status, 403, "{b}: {:?}", no_cookie.body);
        assert_eq!(reason(&no_cookie), "expired", "{b}");
        assert_eq!(reason(&other_account), "expired", "{b}");
        assert_eq!(preview.status, 200, "{b}: {:?}", preview.body);
        assert_eq!(preview.body["provider"], "Corp SSO", "{b}");
        assert_eq!(preview.body["username"], "alice", "{b}");
        assert_eq!(preview.body["subject"], "sub-a", "{b}");
        assert_eq!(reload.body, preview.body, "{b}: a reload answers the same");
        assert_eq!(env.idp.token_requests().await.len(), 1, "{b}: one exchange");
        assert_eq!(reason(&bob_confirms), "expired", "{b}");
        // One verification row (the reload answered from the flow), naming the provider
        // as the other link rows do.
        let verifications: Vec<_> = env
            .h
            .audit_rows()
            .await
            .into_iter()
            .filter(|r| r.action == "user.identity_link_verify")
            .collect();
        assert_eq!(verifications.len(), 1, "{b}");
        let after: Value =
            serde_json::from_str(verifications[0].after.as_deref().unwrap()).unwrap();
        assert_eq!(after["provider"], "corp", "{b}");
        assert_eq!(after["subject"], "sub-a", "{b}");
        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["subject"], "sub-a", "{b}");
        assert_eq!(done.body["linked_via"], "explicit", "{b}");
        assert_eq!(
            cookie_value(&done, OIDC_BINDING_COOKIE).as_deref(),
            Some(""),
            "{b}"
        );
        let identities = account.body["identities"].as_array().unwrap();
        assert_eq!(identities.len(), 1, "{b}");
        assert_eq!(identities[0]["provider"], "corp", "{b}");
        assert_eq!(identities[0]["provider_display_name"], "Corp SSO", "{b}");
        assert!(
            audit_actions(db)
                .await
                .contains(&"user.identity_link".to_owned()),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn refusals_name_their_reason() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.h.console("alice", Role::User).await;
        let bob = env.h.console("bob", Role::User).await;
        env.link(&alice, "sub-a", password()).await;
        let b = db.backend;

        let taken = env.link(&bob, "sub-a", password()).await;
        let second = env.start(&alice, password()).await;
        let olga = external_console(&env.h, "olga", AuthSource::Oidc).await;
        let oidc_account = env.start(&olga, password()).await;
        let lara = external_console(&env.h, "lara", AuthSource::Ldap).await;
        let ldap_account = env.start(&lara, password()).await;

        assert_eq!(taken.status, 403, "{b}: {:?}", taken.body);
        assert_eq!(reason(&taken), "identity_taken", "{b}");
        assert_eq!(reason(&second), "provider_linked", "{b}");
        assert_eq!(oidc_account.status, 409, "{b}: {:?}", oidc_account.body);
        assert_eq!(
            oidc_account.body["error"]["code"], "not_local_account",
            "{b}"
        );
        assert_eq!(reason(&ldap_account), "directory_account", "{b}");
    })
    .await;
}

/// The acceptance case on the console path (the client path is covered in core).
#[tokio::test]
async fn an_owner_with_totp_signs_in_through_the_linked_identity_without_it() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let owner = env.h.console("root", Role::Owner).await;
        let totp = env.h.enrol(owner.user_id).await;
        let proof = json!({ "current_password": PASSWORD, "code": totp.next_code() });
        let linked = env.link(&owner, "sub-r", proof).await;
        assert_eq!(linked.status, 200, "{:?}", linked.body);

        let start = env
            .h
            .post(LOGIN_START, None, json!({ "provider": "corp" }))
            .await;
        let url = url::Url::parse(start.body["url"].as_str().unwrap()).unwrap();
        let request = AuthRequest::parse(&url);
        env.mount("sub-r", &request).await;
        let binding = cookie_value(&start, OIDC_BINDING_COOKIE).unwrap();
        let cookie = format!("{OIDC_BINDING_COOKIE}={binding}");
        let body = json!({ "state": request.state, "code": AUTH_CODE });
        let signed = env
            .h
            .post_with(LOGIN_COMPLETE, None, body, &[("cookie", &cookie)])
            .await;

        let b = db.backend;
        assert_eq!(signed.status, 200, "{b}: {:?}", signed.body);
        assert_eq!(signed.body["status"], "signed_in", "{b}");
        assert_eq!(signed.body["user"]["id"], owner.user_id, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_holder_unlinks_with_the_password() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let alice = env.h.console("alice", Role::User).await;
        let linked = env.link(&alice, "sub-a", password()).await;
        let id = linked.body["id"].as_i64().unwrap();
        let path = format!("/admin/api/v1/me/identities/{id}/unlink");
        let b = db.backend;

        let wrong = env
            .h
            .post(
                &path,
                Some(&alice),
                json!({ "current_password": "nope-nope" }),
            )
            .await;
        let done = env.h.post(&path, Some(&alice), password()).await;

        assert_eq!(wrong.body["error"]["code"], "wrong_password", "{b}");
        assert_eq!(done.status, 204, "{b}: {:?}", done.body);
        assert_eq!(
            user_identity::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "{b}"
        );
        assert!(
            audit_actions(db)
                .await
                .contains(&"user.identity_unlink".to_owned()),
            "{b}"
        );
    })
    .await;
}

/// An administrator sees a user's identities and unlinks them; only an owner touches an
/// owner's; an `oidc` account keeps its last one; a plain user may not.
#[tokio::test]
async fn administrators_unlink_by_the_account_rules() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = env.h.console("adam", Role::Admin).await;
        let owner = env.h.console("root", Role::Owner).await;
        let alice = env.h.console("alice", Role::User).await;
        let alice_link = env.link(&alice, "sub-a", password()).await.body["id"].clone();
        let owner_link = env.link(&owner, "sub-r", password()).await.body["id"].clone();
        let olga = oidc_account(&env.h, "sub-o").await;
        let b = db.backend;
        let unlink = |user: i64, identity: &Value| {
            format!("/admin/api/v1/users/{user}/identities/{identity}")
        };

        let detail = env
            .h
            .as_console(
                Method::GET,
                &format!("/admin/api/v1/users/{}", alice.user_id),
                &admin,
                None,
            )
            .await;
        let by_user = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(alice.user_id, &alice_link),
                &alice,
                None,
            )
            .await;
        let wrong_user = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(owner.user_id, &alice_link),
                &admin,
                None,
            )
            .await;
        let owner_by_admin = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(owner.user_id, &owner_link),
                &admin,
                None,
            )
            .await;
        let last = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(olga.0, &json!(olga.1)),
                &admin,
                None,
            )
            .await;
        let by_admin = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(alice.user_id, &alice_link),
                &admin,
                None,
            )
            .await;
        let owner_by_owner = env
            .h
            .as_console(
                Method::DELETE,
                &unlink(owner.user_id, &owner_link),
                &owner,
                None,
            )
            .await;

        assert_eq!(
            detail.body["identities"][0]["subject"], "sub-a",
            "{b}: {:?}",
            detail.body
        );
        assert_eq!(by_user.status, 403, "{b}");
        assert_eq!(wrong_user.status, 404, "{b}");
        assert_eq!(
            owner_by_admin.body["error"]["code"], "owner_required",
            "{b}"
        );
        assert_eq!(last.status, 409, "{b}: {:?}", last.body);
        assert_eq!(last.body["error"]["code"], "last_identity", "{b}");
        assert_eq!(by_admin.status, 204, "{b}: {:?}", by_admin.body);
        assert_eq!(owner_by_owner.status, 204, "{b}: {:?}", owner_by_owner.body);
        assert_eq!(
            user_identity::Entity::find().count(&db.conn).await.unwrap(),
            1,
            "{b}: only the oidc account's identity is left"
        );
    })
    .await;
}

/// Unlinking an identity, here an explicit one, marks the verified address unverified in the same change,
/// and the audit row says so.
#[tokio::test]
async fn an_admin_unlinking_an_explicit_link_unverifies_the_address() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let admin = env.h.console("adam", Role::Admin).await;
        let new = NewUser {
            email: Some("bob@example.com".to_owned()),
            email_verified: true,
            ..NewUser::new("bob", Credential::Oidc)
        };
        let bob = env.h.state.users.create(new).await.unwrap().id;
        let provider = env.h.state.oidc.list_providers().await.unwrap()[0].id;
        let row = user_identity::ActiveModel {
            user_id: sea_orm::ActiveValue::Set(bob),
            provider_id: sea_orm::ActiveValue::Set(provider),
            subject: sea_orm::ActiveValue::Set("sub-b".to_owned()),
            email_verified: sea_orm::ActiveValue::Set(true),
            email: sea_orm::ActiveValue::Set(Some("bob@example.com".to_owned())),
            created_at: sea_orm::ActiveValue::Set(deskhaven_db::time::now()),
            linked_via: sea_orm::ActiveValue::Set(user_identity::LinkedVia::Explicit),
            ..Default::default()
        };
        let id = sea_orm::ActiveModelTrait::insert(row, &env.h.state.db)
            .await
            .unwrap()
            .id;
        // A second identity, so the oidc account may lose this one.
        oidc_account_identity(&env.h, bob, "sub-b2").await;

        let path = format!("/admin/api/v1/users/{bob}/identities/{id}");
        let reply = env.h.as_console(Method::DELETE, &path, &admin, None).await;

        let b = db.backend;
        assert_eq!(reply.status, 204, "{b}: {:?}", reply.body);
        assert!(
            !env.h.state.users.get(bob).await.unwrap().email_verified,
            "{b}"
        );
        let rows = env.h.audit_rows().await;
        let unlink = rows
            .iter()
            .find(|r| r.action == "user.identity_unlink")
            .unwrap();
        let after: Value = serde_json::from_str(unlink.after.as_deref().unwrap()).unwrap();
        assert_eq!(after, json!({ "email_verified": false }), "{b}");
    })
    .await;
}

async fn oidc_account_identity(h: &Harness, user_id: i64, subject: &str) {
    let mut second = NewProvider::new("second", "https://second.example.com", "c");
    second.client_secret = Some("s".to_owned());
    let provider = h.state.oidc.create_provider(second).await.unwrap().id;
    let row = user_identity::ActiveModel {
        user_id: sea_orm::ActiveValue::Set(user_id),
        provider_id: sea_orm::ActiveValue::Set(provider),
        subject: sea_orm::ActiveValue::Set(subject.to_owned()),
        email_verified: sea_orm::ActiveValue::Set(false),
        email: sea_orm::ActiveValue::Set(None),
        created_at: sea_orm::ActiveValue::Set(deskhaven_db::time::now()),
        linked_via: sea_orm::ActiveValue::Set(user_identity::LinkedVia::Explicit),
        ..Default::default()
    };
    sea_orm::ActiveModelTrait::insert(row, &h.state.db)
        .await
        .unwrap();
}

/// An `oidc` account with one identity (as provisioning leaves it): (user id, identity id).
async fn oidc_account(h: &Harness, subject: &str) -> (i64, i64) {
    let new = NewUser::new(format!("u-{subject}"), Credential::Oidc);
    let user = h.state.users.create(new).await.unwrap();
    let provider = h.state.oidc.list_providers().await.unwrap()[0].id;
    let row = user_identity::ActiveModel {
        user_id: sea_orm::ActiveValue::Set(user.id),
        provider_id: sea_orm::ActiveValue::Set(provider),
        subject: sea_orm::ActiveValue::Set(subject.to_owned()),
        email_verified: sea_orm::ActiveValue::Set(false),
        email: sea_orm::ActiveValue::Set(None),
        created_at: sea_orm::ActiveValue::Set(deskhaven_db::time::now()),
        linked_via: sea_orm::ActiveValue::Set(user_identity::LinkedVia::Provisioned),
        ..Default::default()
    };
    let id = sea_orm::ActiveModelTrait::insert(row, &h.state.db)
        .await
        .unwrap()
        .id;
    (user.id, id)
}
