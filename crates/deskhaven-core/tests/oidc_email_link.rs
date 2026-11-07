//! Automatic linking by email (M4-09 rule 3): an unknown identity links an existing account
//! only when every condition holds: the IdP asserts the address verified, the account's
//! own address is verified, its role is within `auth.oidc.link_email_max_role`, it has no
//! TOTP (whatever the ceiling) and it is not an LDAP account. The matrix runs in one
//! database per backend, one account per case. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, enable_totp};
use common::set;
use deskhaven_core::oidc::Failure;
use deskhaven_core::users::{Credential, NewUser, Password, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::json;

#[derive(Clone, Copy, Debug)]
struct Case {
    max_role: &'static str,
    role: Role,
    totp: bool,
    /// The token's `email_verified` claim.
    idp_verified: Option<bool>,
    local_verified: bool,
    ldap: bool,
}

const BASE: Case = Case {
    max_role: "owner",
    role: Role::User,
    totp: false,
    idp_verified: Some(true),
    local_verified: true,
    ldap: false,
};

fn rank(role: Role) -> u8 {
    match role {
        Role::User => 1,
        Role::Admin => 2,
        Role::Owner => 3,
    }
}

fn ceiling(max_role: &str) -> u8 {
    match max_role {
        "user" => 1,
        "admin" => 2,
        "owner" => 3,
        _ => 0,
    }
}

impl Case {
    fn links(self) -> bool {
        self.idp_verified == Some(true)
            && self.local_verified
            && rank(self.role) <= ceiling(self.max_role)
            && !self.totp
            && !self.ldap
    }
}

/// Every ceiling against every role, then each other condition failing alone at the most
/// permissive ceiling (TOTP also with an `owner` ceiling and a plain user).
fn cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for max_role in ["off", "user", "admin", "owner"] {
        for role in [Role::User, Role::Admin, Role::Owner] {
            cases.push(Case {
                max_role,
                role,
                ..BASE
            });
        }
    }
    for role in [Role::User, Role::Owner] {
        cases.push(Case {
            role,
            totp: true,
            ..BASE
        });
    }
    cases.extend([
        Case {
            idp_verified: Some(false),
            ..BASE
        },
        Case {
            idp_verified: None,
            ..BASE
        },
        Case {
            local_verified: false,
            ..BASE
        },
        Case { ldap: true, ..BASE },
        Case {
            max_role: "user",
            local_verified: false,
            idp_verified: None,
            ..BASE
        },
    ]);
    cases
}

async fn account(env: &Env, db: &TestDb, n: usize, case: Case) -> (i64, String) {
    let email = format!("case{n}@example.com");
    let credential = match case.ldap {
        true => Credential::Ldap,
        false => Credential::Local {
            password: Password::new(common::oidc::PASSWORD),
        },
    };
    let new = NewUser {
        email: Some(email.clone()),
        email_verified: case.local_verified,
        ..NewUser::new(format!("case{n}"), credential)
    };
    let user = env.users.create(new).await.unwrap();
    if case.role != Role::User {
        env.users.set_role(user.id, case.role).await.unwrap();
    }
    if case.totp {
        enable_totp(db, user.id).await;
    }
    (user.id, email)
}

#[tokio::test]
async fn email_links_exactly_when_every_condition_holds() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        for (n, case) in cases().into_iter().enumerate() {
            set(
                &env.settings,
                "auth.oidc.link_email_max_role",
                json!(case.max_role),
            )
            .await;
            let (user_id, email) = account(&env, db, n, case).await;

            let (_, report) = env
                .login(&format!("sub-{n}"), |s| s.email(&email, case.idp_verified))
                .await;

            let b = db.backend;
            match case.links() {
                true => assert_eq!(report.outcome.unwrap().user.id, user_id, "{b}: {case:?}"),
                false => assert_eq!(
                    report.outcome.err(),
                    Some(Failure::NoLinkedAccount),
                    "{b}: {case:?}"
                ),
            }
            let linked = user_identity::Entity::find()
                .filter(user_identity::Column::UserId.eq(user_id))
                .one(&db.conn)
                .await
                .unwrap();
            assert_eq!(linked.is_some(), case.links(), "{b}: {case:?}");
            if let Some(linked) = linked {
                assert_eq!(linked.linked_via, LinkedVia::Email, "{b}: {case:?}");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn the_default_ceiling_links_plain_users_only() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, email) = account(&env, db, 0, BASE).await;
        let (_, admin_email) = account(
            &env,
            db,
            1,
            Case {
                role: Role::Admin,
                ..BASE
            },
        )
        .await;

        let user = env.login("sub-u", |s| s.email(&email, Some(true))).await.1;
        let admin = env
            .login("sub-a", |s| s.email(&admin_email, Some(true)))
            .await
            .1;

        let b = db.backend;
        assert_eq!(user.outcome.unwrap().user.id, user_id, "{b}");
        assert_eq!(admin.outcome.err(), Some(Failure::NoLinkedAccount), "{b}");
    })
    .await;
}

/// `auto_provision` does not change any of it: an address that belongs to an account that
/// may not be linked is refused, never provisioned a second time (rule 6).
#[tokio::test]
async fn provisioning_never_creates_a_second_account_for_a_refused_address() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (_, email) = account(&env, db, 0, Case { totp: true, ..BASE }).await;

        let report = env.login("sub-t", |s| s.email(&email, Some(true))).await.1;

        let b = db.backend;
        assert_eq!(report.outcome.err(), Some(Failure::NoLinkedAccount), "{b}");
        assert!(
            user_identity::Entity::find()
                .one(&db.conn)
                .await
                .unwrap()
                .is_none(),
            "{b}"
        );
    })
    .await;
}

/// Identities the login itself creates are audited with the system as the actor: an email
/// link as `user.identity_link` (`linked_via: email`), a provisioned account as
/// `user.create`, each naming the provider and no claim value beyond the subject.
#[tokio::test]
async fn email_links_and_provisioning_are_audited_as_the_system() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (user_id, email) = account(&env, db, 0, BASE).await;
        env.login("sub-e", |s| s.email(&email, Some(true)))
            .await
            .1
            .outcome
            .unwrap();
        let olga = env
            .login("sub-o", |s| s.email("olga@corp.example", Some(true)))
            .await
            .1;
        let olga = olga.outcome.unwrap().user.id;

        let rows = console_audit::Entity::find()
            .order_by_asc(console_audit::Column::Id)
            .all(&db.conn)
            .await
            .unwrap();

        let b = db.backend;
        let summary: Vec<(&str, String)> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.target_id.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                ("user.identity_link", user_id.to_string()),
                ("user.create", olga.to_string())
            ],
            "{b}"
        );
        for row in &rows {
            assert_eq!(row.actor_kind, console_audit::ActorKind::System, "{b}");
            let after = row.after.as_deref().unwrap();
            assert!(after.contains("\"provider\":\"corp\""), "{b}: {after}");
            // The email link names no claim value but the subject (the provisioned row also
            // names the new account, whose username may come from a claim).
            let addresses = ["case0@example.com"];
            assert!(!addresses.iter().any(|a| after.contains(a)), "{b}: {after}");
        }
        assert!(
            rows[0]
                .after
                .as_deref()
                .unwrap()
                .contains("\"linked_via\":\"email\""),
            "{b}"
        );
    })
    .await;
}

/// One identity per provider: a second `sub` with the same verified address (a recreated
/// IdP account) never joins an account that already has an identity there.
#[tokio::test]
async fn a_second_subject_with_the_same_address_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, email) = account(&env, db, 0, BASE).await;
        env.link(user_id, "sub-1", |s| s).await.outcome.unwrap();

        let other = env.login("sub-2", |s| s.email(&email, Some(true))).await.1;

        let b = db.backend;
        assert_eq!(other.outcome.err(), Some(Failure::NoLinkedAccount), "{b}");
        let rows = user_identity::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(rows.len(), 1, "{b}");
    })
    .await;
}
