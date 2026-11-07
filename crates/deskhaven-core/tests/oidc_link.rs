//! Explicit linking (M4-09): a signed-in user links an IdP identity to their own account
//! through a `link` flow bound to the user and the browser that started it. The email plays
//! no part; refused: an identity linked elsewhere, a second identity at the same provider,
//! LDAP accounts. An identity linked by email is upgraded to `explicit`. Once linked, the
//! account signs in through the IdP on both paths without its local TOTP. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PROVIDER, enable_totp};
use common::oidc_idp::{AUTH_CODE, AuthRequest};
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, LinkFailure, LinkReport, OidcError, PreviewTarget,
};
use deskhaven_core::users::Role;
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{console_audit, oidc_flow};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

fn params(request: &AuthRequest) -> CallbackParams<'_> {
    CallbackParams {
        state: Some(&request.state),
        code: Some(AUTH_CODE),
        error: None,
    }
}

fn failure(report: &LinkReport) -> Option<LinkFailure> {
    report.outcome.as_ref().err().copied()
}

async fn identities(db: &TestDb) -> Vec<user_identity::Model> {
    user_identity::Entity::find().all(&db.conn).await.unwrap()
}

async fn flows(db: &TestDb) -> u64 {
    oidc_flow::Entity::find().count(&db.conn).await.unwrap()
}

#[tokio::test]
async fn the_signed_in_user_links_an_identity_whatever_its_email() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", Some("alice@example.com")).await;

        let report = env
            .link(alice.id, "sub-a", |s| {
                s.email("someone@else.example", Some(false))
            })
            .await;

        let b = db.backend;
        let linked = report.outcome.unwrap();
        assert_eq!(linked.user_id, alice.id, "{b}");
        assert_eq!(linked.subject, "sub-a", "{b}");
        assert_eq!(linked.provider_name, PROVIDER, "{b}");
        assert_eq!(linked.linked_via, LinkedVia::Explicit, "{b}");
        assert_eq!(report.subject.as_deref(), Some("sub-a"), "{b}");
        let rows = identities(db).await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(
            rows[0].email, None,
            "{b}: an unverified address is never stored"
        );
        assert_eq!(flows(db).await, 0, "{b}: the flow is gone");
        let audit = console_audit::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(audit.len(), 1, "{b}");
        assert_eq!(audit[0].action, "user.identity_link", "{b}");
        assert_eq!(audit[0].target_id, alice.id.to_string(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn only_the_starting_browser_and_user_complete_a_link_flow() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let bob = env.local_user("bob", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let other = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-a", &request)).await;
        let binding = start.binding.expose();
        let guard = || env.link_guard(alice.id, "user.identity_link");

        let none = env
            .oidc
            .complete_link(&params(&request), None, alice.id, guard())
            .await;
        let wrong = env
            .oidc
            .complete_link(
                &params(&request),
                Some(other.binding.expose()),
                alice.id,
                guard(),
            )
            .await;
        let other_user = env
            .oidc
            .complete_link(&params(&request), Some(binding), bob.id, guard())
            .await;
        let as_login = env
            .oidc
            .complete_console(&params(&request), Some(binding))
            .await;
        let as_client = env.oidc.complete_callback(&params(&request)).await;
        let preview_other = env
            .oidc
            .preview_link(Some(&request.state), Some(binding), bob.id)
            .await;
        let preview = env
            .oidc
            .preview_link(Some(&request.state), Some(binding), alice.id)
            .await
            .unwrap();
        let done = env
            .oidc
            .complete_link(&params(&request), Some(binding), alice.id, guard())
            .await
            .unwrap();

        let b = db.backend;
        for refused in [none, wrong, other_user] {
            assert!(matches!(refused, Err(CallbackError::InvalidState)), "{b}");
        }
        assert!(matches!(as_login, Err(CallbackError::InvalidState)), "{b}");
        assert!(matches!(as_client, Err(CallbackError::InvalidState)), "{b}");
        assert!(
            matches!(preview_other, Err(CallbackError::InvalidState)),
            "{b}"
        );
        assert_eq!(preview.provider, env.provider.display_name, "{b}");
        assert_eq!(done.outcome.unwrap().user_id, alice.id, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_callback_preview_sends_a_link_flow_to_the_console() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);

        let preview = env
            .oidc
            .preview_callback(Some(&request.state))
            .await
            .unwrap();

        assert_eq!(preview.target, PreviewTarget::Link, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn an_identity_of_another_account_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let olga = env.login("sub-o", |s| s).await.1.outcome.unwrap().user;
        let alice = env.local_user("alice", None).await;

        let report = env.link(alice.id, "sub-o", |s| s).await;

        let b = db.backend;
        assert_eq!(failure(&report), Some(LinkFailure::IdentityTaken), "{b}");
        let rows = identities(db).await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].user_id, olga.id, "{b}");
        assert_eq!(
            flows(db).await,
            1,
            "{b}: the link flow is gone, the client login waits"
        );
        assert_eq!(
            console_audit::Entity::find().count(&db.conn).await.unwrap(),
            0,
            "{b}: a refused link is not audited as one"
        );
    })
    .await;
}

#[tokio::test]
async fn a_second_identity_at_the_same_provider_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        env.link(alice.id, "sub-a", |s| s).await.outcome.unwrap();

        let again = env.oidc.begin_link(PROVIDER, alice.id).await;

        let b = db.backend;
        assert!(
            matches!(
                again,
                Err(OidcError::LinkRefused(LinkFailure::ProviderLinked))
            ),
            "{b}: {again:?}"
        );
        assert_eq!(identities(db).await.len(), 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_concurrent_second_link_at_the_same_provider_is_refused_at_completion() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        env.link(alice.id, "sub-a", |s| s).await.outcome.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-b", &request)).await;

        let report = env
            .oidc
            .complete_link(
                &params(&request),
                Some(start.binding.expose()),
                alice.id,
                env.link_guard(alice.id, "user.identity_link"),
            )
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(failure(&report), Some(LinkFailure::ProviderLinked), "{b}");
        assert_eq!(identities(db).await.len(), 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn ldap_accounts_cannot_link() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let lara = env.ldap_user("lara", "lara@example.com").await;

        let refused = env.oidc.begin_link(PROVIDER, lara.id).await;

        assert!(
            matches!(
                refused,
                Err(OidcError::LinkRefused(LinkFailure::DirectoryAccount))
            ),
            "{}: {refused:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn linking_an_email_linked_identity_again_makes_it_explicit() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", Some("alice@example.com")).await;
        let email = |s: common::oidc_idp::TokenSpec| s.email("alice@example.com", Some(true));
        env.login("sub-a", email).await.1.outcome.unwrap();
        assert_eq!(identities(db).await[0].linked_via, LinkedVia::Email);

        let report = env.link(alice.id, "sub-a", email).await;

        let b = db.backend;
        assert_eq!(
            report.outcome.unwrap().linked_via,
            LinkedVia::Explicit,
            "{b}"
        );
        let rows = identities(db).await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].linked_via, LinkedVia::Explicit, "{b}");
    })
    .await;
}

/// The acceptance case: an owner with TOTP links explicitly, then both OIDC paths sign in
/// without asking for the local second factor.
#[tokio::test]
async fn an_owner_with_totp_signs_in_through_an_explicit_link_on_both_paths() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let owner = env.local_user("root", Some("root@example.com")).await;
        env.users.set_role(owner.id, Role::Owner).await.unwrap();
        enable_totp(db, owner.id).await;
        env.link(owner.id, "sub-r", |s| s).await.outcome.unwrap();

        let client = env.login("sub-r", |s| s).await.1;
        let console = env.console_login("sub-r", |s| s).await;

        let b = db.backend;
        assert_eq!(client.outcome.unwrap().user.id, owner.id, "{b}: client");
        assert_eq!(console.outcome.unwrap().user.id, owner.id, "{b}: console");
    })
    .await;
}
