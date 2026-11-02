//! Who may manage whom (06 §4.1): every user endpoint, as user / admin / owner, against an
//! owner / admin / user target. Users never pass the gate; admins manage admins and users
//! but no owner (`owner_required`, nothing changes, no audit row; reading one is fine);
//! owners manage everyone. Promotion to owner and creating owners need an owner. Runs on
//! SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

#[derive(Clone, Copy, Debug)]
enum Action {
    Get,
    Update,
    Disable,
    Enable,
    ResetPassword,
    Reset2fa,
    RevokeSessions,
    Unlock,
    Delete,
}

const ACTIONS: [Action; 9] = [
    Action::Get,
    Action::Update,
    Action::Disable,
    Action::Enable,
    Action::ResetPassword,
    Action::Reset2fa,
    Action::RevokeSessions,
    Action::Unlock,
    Action::Delete,
];
const ROLES: [Role; 3] = [Role::User, Role::Admin, Role::Owner];

impl Action {
    fn writes(self) -> bool {
        !matches!(self, Self::Get)
    }

    async fn run(self, h: &Harness, actor: &Console, target: i64) -> (u16, String) {
        let base = format!("/admin/api/v1/users/{target}");
        let post = |suffix: &str| format!("{base}/{suffix}");
        let reply = match self {
            Self::Get => h.get(&base, actor).await,
            Self::Update => {
                let body = json!({ "display_name": "changed" });
                h.write(Method::PATCH, &base, actor, body).await
            }
            Self::Disable => h.act(Method::POST, &post("disable"), actor).await,
            Self::Enable => h.act(Method::POST, &post("enable"), actor).await,
            Self::ResetPassword => h.act(Method::POST, &post("reset-password"), actor).await,
            Self::Reset2fa => h.act(Method::POST, &post("reset-2fa"), actor).await,
            Self::RevokeSessions => h.act(Method::POST, &post("revoke-sessions"), actor).await,
            Self::Unlock => h.act(Method::POST, &post("unlock"), actor).await,
            Self::Delete => h.act(Method::DELETE, &base, actor).await,
        };
        (reply.status.as_u16(), reply.code().to_owned())
    }
}

/// `None`: allowed (2xx); `Some((status, code))`: refused with this error. Reading an
/// owner account is not managing it.
fn expected(actor: Role, target: Role, action: Action) -> Option<(u16, &'static str)> {
    match (actor, target) {
        (Role::User, _) => Some((403, "forbidden")),
        (Role::Admin, Role::Owner) if action.writes() => Some((403, "owner_required")),
        _ => None,
    }
}

#[tokio::test]
async fn every_action_follows_the_role_matrix() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        // A second owner keeps every owner-target action clear of the last-owner rule.
        h.create_user("keeper", Role::Owner).await;
        let actors = [
            h.console("as-user", Role::User).await,
            h.console("as-admin", Role::Admin).await,
            h.console("as-owner", Role::Owner).await,
        ];
        let mut n = 0;
        for (actor, actor_role) in actors.iter().zip(ROLES) {
            for target_role in ROLES {
                for action in ACTIONS {
                    n += 1;
                    let target = h.create_user(&format!("t{n}"), target_role).await;
                    let before = h.state.users.get(target).await.unwrap();
                    let audits = h.audit_rows().await.len();
                    let got = action.run(&h, actor, target).await;
                    let cell = format!("{actor_role:?} -> {target_role:?}: {action:?}");
                    match expected(actor_role, target_role, action) {
                        None => {
                            assert!((200..300).contains(&got.0), "{cell}: {got:?}");
                            let recorded = h.audit_rows().await.len() - audits;
                            assert_eq!(recorded, usize::from(action.writes()), "{cell}");
                        }
                        Some((status, code)) => {
                            assert_eq!(got, (status, code.to_owned()), "{cell}");
                            let after = h.state.users.get(target).await.unwrap();
                            assert_eq!(after, before, "{cell}: changed");
                            assert_eq!(h.audit_rows().await.len(), audits, "{cell}");
                        }
                    }
                }
            }
        }
    })
    .await;
}

#[tokio::test]
async fn only_owners_create_owners_or_promote_to_owner() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let owner = h.console("own", Role::Owner).await;
        let admin = h.console("adm", Role::Admin).await;
        let create = |username: &str| json!({ "username": username, "role": "owner" });

        let refused = h
            .write(Method::POST, "/admin/api/v1/users", &admin, create("o1"))
            .await;
        assert_eq!(
            (refused.status.as_u16(), refused.code()),
            (403, "owner_required")
        );
        assert!(h.state.users.get_by_username("o1").await.is_err());
        let created = h
            .write(Method::POST, "/admin/api/v1/users", &owner, create("o2"))
            .await;
        assert_eq!(created.status, 201);

        let bob = h.create_user("bob", Role::User).await;
        let path = format!("/admin/api/v1/users/{bob}");
        let promote = json!({ "role": "owner" });
        let refused = h.write(Method::PATCH, &path, &admin, promote.clone()).await;
        assert_eq!(refused.code(), "owner_required");
        assert_eq!(h.state.users.get(bob).await.unwrap().role, Role::User);
        // Admins do manage admins: promotion to admin and back is theirs.
        let to_admin = h
            .write(Method::PATCH, &path, &admin, json!({ "role": "admin" }))
            .await;
        assert_eq!(to_admin.status, 200);
        let promoted = h.write(Method::PATCH, &path, &owner, promote).await;
        assert_eq!(
            (promoted.status.as_u16(), promoted.body["role"].as_str()),
            (200, Some("owner"))
        );

        // A user is stopped at the gate, before any rule.
        let user = h.console("usr", Role::User).await;
        let denied = h
            .write(Method::POST, "/admin/api/v1/users", &user, create("o3"))
            .await;
        assert_eq!((denied.status.as_u16(), denied.code()), (403, "forbidden"));
        let listed = h.get("/admin/api/v1/users", &user).await;
        assert_eq!(listed.status, 403);
        let imported = h
            .write(
                Method::POST,
                "/admin/api/v1/users/import",
                &user,
                json!({ "csv": "username\nx" }),
            )
            .await;
        assert_eq!(imported.status, 403);
    })
    .await;
}
