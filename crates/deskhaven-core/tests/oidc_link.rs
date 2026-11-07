//! Explicit linking (M4-09): a signed-in user links an IdP identity to their own account
//! through a `link` flow bound to the user and the browser that started it. The email plays
//! no part; refused: an identity linked elsewhere, a second identity at the same provider,
//! LDAP accounts. An identity linked by email is upgraded to `explicit`. Once linked, the
//! account signs in through the IdP on both paths without its local TOTP. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PROVIDER, enable_totp};
use common::oidc_idp::{AUTH_CODE, AuthRequest, TokenSpec};
use deskhaven_core::oidc::{
    CallbackError, CallbackParams, LinkFailure, LinkPending, LinkReport, OidcError, PreviewTarget,
};
use deskhaven_core::users::Role;
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{console_audit, oidc_flow};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

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
async fn only_the_starting_browser_and_user_verify_and_complete_a_link_flow() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let bob = env.local_user("bob", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let other = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-a", &request)).await;
        let binding = start.binding.expose();
        let state = Some(request.state.as_str());
        let guard = || env.link_guard(alice.id, "user.identity_link");
        let answer = params(&request);
        let verify = |binding, user| env.oidc.verify_link(&answer, binding, user);

        let unverified = env
            .oidc
            .complete_link(state, Some(binding), alice.id, guard())
            .await;
        let no_cookie = verify(None, alice.id).await;
        let wrong = verify(Some(other.binding.expose()), alice.id).await;
        let other_user = verify(Some(binding), bob.id).await;
        let as_login = env
            .oidc
            .complete_console(&params(&request), Some(binding))
            .await;
        let as_client = env.oidc.complete_callback(&params(&request)).await;
        let verified = verify(Some(binding), alice.id).await.unwrap().unwrap();
        let none = env.oidc.complete_link(state, None, alice.id, guard()).await;
        let by_bob = env
            .oidc
            .complete_link(state, Some(binding), bob.id, guard())
            .await;
        let done = env
            .oidc
            .complete_link(state, Some(binding), alice.id, guard())
            .await;
        let again = env
            .oidc
            .complete_link(state, Some(binding), alice.id, guard())
            .await;

        let b = db.backend;
        let invalid = |r: &Result<_, CallbackError>| matches!(r, Err(CallbackError::InvalidState));
        assert!(invalid(&unverified), "{b}: not verified yet");
        for refused in [&no_cookie, &wrong, &other_user] {
            assert!(matches!(refused, Err(CallbackError::InvalidState)), "{b}");
        }
        assert!(matches!(as_login, Err(CallbackError::InvalidState)), "{b}");
        assert!(matches!(as_client, Err(CallbackError::InvalidState)), "{b}");
        assert_eq!(verified.provider, env.provider.display_name, "{b}");
        assert_eq!(verified.subject, "sub-a", "{b}");
        assert!(invalid(&none) && invalid(&by_bob), "{b}");
        assert_eq!(done.unwrap().outcome.unwrap().user_id, alice.id, "{b}");
        assert!(invalid(&again), "{b}: single use");
    })
    .await;
}

/// The confirmation names the IdP identity: verification exchanges the code once, a reload
/// answers from the flow, and confirming links exactly the identity shown.
#[tokio::test]
async fn verification_exchanges_once_and_the_confirmation_links_what_it_showed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        let spec = TokenSpec {
            preferred_username: Some("alice.corp".to_owned()),
            ..env
                .spec("sub-a", &request)
                .email("alice@corp.example", Some(true))
        };
        env.idp.mount_token(&spec).await;
        let binding = Some(start.binding.expose());

        let first = env
            .oidc
            .verify_link(&params(&request), binding, alice.id)
            .await;
        let reload = env
            .oidc
            .verify_link(&params(&request), binding, alice.id)
            .await;
        let guard = env.link_guard(alice.id, "user.identity_link");
        let done = env
            .oidc
            .complete_link(Some(&request.state), binding, alice.id, guard)
            .await
            .unwrap();

        let b = db.backend;
        let shown = first.unwrap().unwrap();
        assert_eq!(shown.subject, "sub-a", "{b}");
        assert_eq!(shown.email.as_deref(), Some("alice@corp.example"), "{b}");
        assert_eq!(
            shown.preferred_username.as_deref(),
            Some("alice.corp"),
            "{b}"
        );
        let reload = reload.unwrap().unwrap();
        assert!(
            shown.exchanged && !reload.exchanged,
            "{b}: answered from the flow"
        );
        assert_eq!(
            LinkPending {
                exchanged: true,
                ..reload
            },
            shown,
            "{b}"
        );
        assert_eq!(env.idp.token_requests().await.len(), 1, "{b}: one exchange");
        let linked = done.outcome.unwrap();
        assert_eq!(linked.subject, "sub-a", "{b}");
        assert_eq!(linked.email.as_deref(), Some("alice@corp.example"), "{b}");
    })
    .await;
}

/// A refusal at the IdP is answered at verification, again on a reload, and the flow can
/// never be confirmed.
#[tokio::test]
async fn an_idp_refusal_is_reported_at_verification_and_never_confirmed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        let denied = CallbackParams {
            state: Some(&request.state),
            code: None,
            error: Some("access_denied"),
        };
        let binding = Some(start.binding.expose());

        let first = env
            .oidc
            .verify_link(&denied, binding, alice.id)
            .await
            .unwrap();
        let reload = env
            .oidc
            .verify_link(&params(&request), binding, alice.id)
            .await
            .unwrap();
        let guard = env.link_guard(alice.id, "user.identity_link");
        let confirm = env
            .oidc
            .complete_link(Some(&request.state), binding, alice.id, guard)
            .await;

        let b = db.backend;
        assert_eq!(first.err(), Some(LinkFailure::Denied), "{b}");
        assert_eq!(reload.err(), Some(LinkFailure::Denied), "{b}");
        assert!(matches!(confirm, Err(CallbackError::InvalidState)), "{b}");
        assert_eq!(env.idp.token_requests().await.len(), 0, "{b}");
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
            console_audit::Entity::find()
                .filter(console_audit::Column::Action.eq("user.identity_link"))
                .count(&db.conn)
                .await
                .unwrap(),
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

        let binding = Some(start.binding.expose());
        env.oidc
            .verify_link(&params(&request), binding, alice.id)
            .await
            .unwrap()
            .unwrap();
        let guard = env.link_guard(alice.id, "user.identity_link");
        let report = env
            .oidc
            .complete_link(Some(&request.state), binding, alice.id, guard)
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
