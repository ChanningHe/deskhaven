//! The per-route gate: console session cookie, CSRF, role (M3-01 acceptance).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, harness, probe};
use deskhaven_admin_api::{CSRF_COOKIE, SESSION_COOKIE, session_cookies};
use deskhaven_core::users::{Role, Status};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

async fn app(db: &TestDb) -> Harness {
    harness(db, probe::routes()).await
}

#[tokio::test]
async fn me_returns_the_signed_in_user_for_every_role() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        for (name, role, wire) in [
            ("uma", Role::User, "user"),
            ("ada", Role::Admin, "admin"),
            ("olga", Role::Owner, "owner"),
        ] {
            let console = app.console(name, role).await;

            let reply = app.get("/admin/api/v1/me", &console).await;

            assert_eq!(
                reply.status,
                StatusCode::OK,
                "{}: {:?}",
                db.backend,
                reply.body
            );
            assert_eq!(
                reply.body,
                json!({
                    "id": console.user_id,
                    "username": name,
                    "display_name": name,
                    "email": null,
                    "role": wire,
                    "must_change_password": false,
                    "totp_enabled": false,
                })
            );
            assert_eq!(reply.headers["cache-control"], "no-store");
        }
    })
    .await;
}

#[tokio::test]
async fn missing_malformed_or_unknown_cookie_is_401() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let unknown = format!("{SESSION_COOKIE}={}", "A".repeat(43));
        let cookies = [
            None,
            Some("other=1".to_owned()),
            Some(format!("{SESSION_COOKIE}=garbage")),
            Some(unknown),
        ];
        for cookie in cookies {
            let reply = app
                .send(
                    Method::GET,
                    "/admin/api/v1/me",
                    cookie.as_deref(),
                    None,
                    None,
                )
                .await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{cookie:?}");
            assert_eq!(reply.code(), "unauthorized");
        }
    })
    .await;
}

#[tokio::test]
async fn client_kind_session_cookie_is_401() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let cookie = app.client_session_cookie("cleo").await;

        let reply = app
            .send(Method::GET, "/admin/api/v1/me", Some(&cookie), None, None)
            .await;

        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        assert_eq!(reply.code(), "unauthorized");
    })
    .await;
}

#[tokio::test]
async fn revoked_or_disabled_sessions_are_401() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;
        let ben = app.console("ben", Role::Admin).await;
        assert_eq!(
            app.get("/admin/api/v1/me", &ada).await.status,
            StatusCode::OK
        );

        app.state
            .users
            .set_status(ada.user_id, Status::Disabled)
            .await
            .unwrap();
        app.state
            .sessions
            .revoke_all_for_user(ben.user_id)
            .await
            .unwrap();

        for console in [&ada, &ben] {
            let reply = app.get("/admin/api/v1/me", console).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn state_changing_request_without_or_with_wrong_csrf_is_403() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;
        let other = app.console("ben", Role::Admin).await;
        let path = "/admin/api/v1/probe/admin";

        let cases = [None, Some("wrong"), Some(other.csrf.as_str()), Some("")];
        for csrf in cases {
            let reply = app
                .send(Method::POST, path, Some(&ada.cookie), csrf, Some(json!({})))
                .await;
            assert_eq!(reply.status, StatusCode::FORBIDDEN, "{csrf:?}");
            assert_eq!(reply.code(), "csrf_failed");
        }
        let ok = app.write(Method::POST, path, &ada, json!({})).await;
        assert_eq!(ok.status, StatusCode::OK, "{}: {:?}", db.backend, ok.body);
    })
    .await;
}

#[tokio::test]
async fn safe_methods_need_no_csrf_header() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;

        for method in [Method::GET, Method::HEAD] {
            let reply = app
                .send(
                    method.clone(),
                    "/admin/api/v1/me",
                    Some(&uma.cookie),
                    None,
                    None,
                )
                .await;
            assert_eq!(reply.status, StatusCode::OK, "{method}");
        }
    })
    .await;
}

#[tokio::test]
async fn roles_below_the_route_permission_are_403() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let ada = app.console("ada", Role::Admin).await;
        let olga = app.console("olga", Role::Owner).await;
        let admin = "/admin/api/v1/probe/admin";
        let owner = "/admin/api/v1/probe/owner";
        let cases = [
            (&uma, Method::POST, admin, StatusCode::FORBIDDEN),
            (&ada, Method::POST, admin, StatusCode::OK),
            (&olga, Method::POST, admin, StatusCode::OK),
            (&uma, Method::DELETE, owner, StatusCode::FORBIDDEN),
            (&ada, Method::DELETE, owner, StatusCode::FORBIDDEN),
            (&olga, Method::DELETE, owner, StatusCode::OK),
        ];
        for (console, method, path, expected) in cases {
            let reply = app.write(method.clone(), path, console, json!({})).await;
            assert_eq!(
                reply.status, expected,
                "{} {method} {path}",
                console.user_id
            );
            if expected == StatusCode::FORBIDDEN {
                assert_eq!(reply.code(), "forbidden");
            }
        }
    })
    .await;
}

#[tokio::test]
async fn console_user_on_a_public_route_still_requires_csrf_for_writes() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let path = "/admin/api/v1/probe/public-session";

        let anonymous = app
            .send(Method::POST, path, None, Some("any"), Some(json!({})))
            .await;
        let no_csrf = app
            .send(Method::POST, path, Some(&uma.cookie), None, Some(json!({})))
            .await;
        let wrong_csrf = app
            .send(
                Method::POST,
                path,
                Some(&uma.cookie),
                Some("any"),
                Some(json!({})),
            )
            .await;
        let ok = app.write(Method::POST, path, &uma, json!({})).await;

        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        for reply in [&no_csrf, &wrong_csrf] {
            assert_eq!(
                (reply.status, reply.code()),
                (StatusCode::FORBIDDEN, "csrf_failed")
            );
        }
        assert_eq!(ok.status, StatusCode::OK);
        assert_eq!(ok.body, json!({ "user_id": uma.user_id }));
    })
    .await;
}

#[tokio::test]
async fn public_writes_need_a_csrf_header_but_no_session() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let path = "/admin/api/v1/probe/public";

        let missing = app
            .send(Method::POST, path, None, None, Some(json!({})))
            .await;
        let empty = app
            .send(Method::POST, path, None, Some(""), Some(json!({})))
            .await;
        let any = app
            .send(Method::POST, path, None, Some("x"), Some(json!({})))
            .await;

        for reply in [&missing, &empty] {
            assert_eq!(
                (reply.status, reply.code()),
                (StatusCode::FORBIDDEN, "csrf_failed")
            );
        }
        assert_eq!(any.status, StatusCode::OK, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn cross_site_writes_are_refused_on_public_and_authenticated_routes() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let ada = app.console("ada", Role::Admin).await;
        let host = [("host", "console.local:8443")];
        let cases: [(&[(&str, &str)], StatusCode); 7] = [
            (&[("sec-fetch-site", "cross-site")], StatusCode::FORBIDDEN),
            (&[("sec-fetch-site", "same-site")], StatusCode::FORBIDDEN),
            (
                &[
                    ("sec-fetch-site", "same-origin"),
                    ("origin", "https://evil.test"),
                ],
                StatusCode::OK,
            ),
            (&[("sec-fetch-site", "none")], StatusCode::OK),
            (
                &[("origin", "https://evil.test"), host[0]],
                StatusCode::FORBIDDEN,
            ),
            (&[("origin", "null"), host[0]], StatusCode::FORBIDDEN),
            (
                &[("origin", "https://console.local:8443"), host[0]],
                StatusCode::OK,
            ),
        ];
        for (extra, expected) in cases {
            let admin = common::request(
                Method::POST,
                "/admin/api/v1/probe/admin",
                &common::auth_headers(&ada, extra),
                Some(json!({})),
            );
            let mut public_headers = vec![("x-deskhaven-csrf", "x")];
            public_headers.extend_from_slice(extra);
            let public = common::request(
                Method::POST,
                "/admin/api/v1/probe/public",
                &public_headers,
                Some(json!({})),
            );
            for (route, req) in [("admin", admin), ("public", public)] {
                let reply = app.raw(req).await;
                assert_eq!(reply.status, expected, "{} {route} {extra:?}", db.backend);
                if expected == StatusCode::FORBIDDEN {
                    assert_eq!(reply.code(), "csrf_failed");
                }
            }
        }
        // Reads are not subject to the check.
        let read = app
            .raw(common::request(
                Method::GET,
                "/admin/api/v1/me",
                &common::auth_headers(&ada, &[("sec-fetch-site", "cross-site")]),
                None,
            ))
            .await;
        assert_eq!(read.status, StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn split_permissions_on_one_path_are_enforced_per_method() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let olga = app.console("olga", Role::Owner).await;
        let path = "/admin/api/v1/probe/split";

        let user_get = app.get(path, &uma).await;
        let user_head = app
            .send(Method::HEAD, path, Some(&uma.cookie), None, None)
            .await;
        let user_post = app.write(Method::POST, path, &uma, json!({})).await;
        let owner_post = app.write(Method::POST, path, &olga, json!({})).await;

        assert_eq!(user_get.body, json!({ "read": true }), "{}", db.backend);
        assert_eq!(user_head.status, StatusCode::OK);
        assert_eq!(
            (user_post.status, user_post.code()),
            (StatusCode::FORBIDDEN, "forbidden")
        );
        assert_eq!(owner_post.body, json!({ "written": true }));
    })
    .await;
}

#[tokio::test]
async fn head_runs_under_the_get_gate() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let ada = app.console("ada", Role::Admin).await;
        let path = "/admin/api/v1/probe/secret";

        let anonymous = app.send(Method::HEAD, path, None, None, None).await;
        let user = app
            .send(Method::HEAD, path, Some(&uma.cookie), None, None)
            .await;
        let admin = app
            .send(Method::HEAD, path, Some(&ada.cookie), None, None)
            .await;

        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        assert_eq!(user.status, StatusCode::FORBIDDEN);
        assert_eq!(admin.status, StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn unserved_methods_pass_the_gate_before_the_405() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let path = "/admin/api/v1/me";

        let anonymous = app.send(Method::DELETE, path, None, Some("x"), None).await;
        let no_csrf = app
            .send(Method::DELETE, path, Some(&uma.cookie), None, None)
            .await;
        let signed_in = app.write(Method::DELETE, path, &uma, json!({})).await;

        assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        assert_eq!(no_csrf.code(), "csrf_failed");
        assert_eq!(
            (signed_in.status, signed_in.code()),
            (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
        );
    })
    .await;
}

#[tokio::test]
async fn duplicate_session_cookies_are_401() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        let olga = app.console("olga", Role::Owner).await;
        let both = format!("{}; {}", uma.cookie, olga.cookie);

        let one_header = app
            .send(Method::GET, "/admin/api/v1/me", Some(&both), None, None)
            .await;
        let two_headers = app
            .raw(common::request(
                Method::GET,
                "/admin/api/v1/me",
                &[
                    ("cookie", uma.cookie.as_str()),
                    ("cookie", olga.cookie.as_str()),
                ],
                None,
            ))
            .await;

        for reply in [one_header, two_headers] {
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn cookie_spraying_is_rate_limited_per_address() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let uma = app.console("uma", Role::User).await;
        // Warm the cache: a cached valid session is never throttled.
        assert_eq!(
            app.get("/admin/api/v1/me", &uma).await.status,
            StatusCode::OK
        );

        let mut limited = None;
        for i in 0..40 {
            let guess = format!("{SESSION_COOKIE}={:A>43}", i);
            let reply = app
                .send(Method::GET, "/admin/api/v1/me", Some(&guess), None, None)
                .await;
            if reply.status == StatusCode::TOO_MANY_REQUESTS {
                limited = Some(reply);
                break;
            }
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{i}");
        }

        let limited = limited.unwrap_or_else(|| panic!("{}: never rate limited", db.backend));
        assert_eq!(limited.code(), "rate_limited");
        assert!(limited.headers.contains_key("retry-after"));
        assert_eq!(
            app.get("/admin/api/v1/me", &uma).await.status,
            StatusCode::OK
        );
    })
    .await;
}

#[tokio::test]
async fn session_cookies_carry_the_required_attributes_and_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let app = app(db).await;
        let user = app.console("uma", Role::User).await;
        let token = user
            .cookie
            .trim_start_matches(&format!("{SESSION_COOKIE}="));
        // Re-mint cookies from a real token through the public helper.
        let auth = app
            .state
            .users
            .authenticate_local(
                "uma",
                &deskhaven_core::users::Password::new(common::PASSWORD),
            )
            .await
            .unwrap();
        let (_, fresh) = app
            .state
            .sessions
            .create_console(user.user_id, &auth.stamp, None, None)
            .await
            .unwrap();

        let [session, csrf] = session_cookies(&fresh, std::time::Duration::from_secs(3600));

        let session = session.to_str().unwrap();
        let csrf = csrf.to_str().unwrap();
        assert!(session.starts_with(&format!("{SESSION_COOKIE}={}; ", fresh.expose())));
        for attr in [
            "Path=/",
            "Secure",
            "SameSite=Lax",
            "HttpOnly",
            "Max-Age=3600",
        ] {
            assert!(session.split("; ").any(|a| a == attr), "{attr}: {session}");
        }
        assert!(csrf.starts_with(&format!("{CSRF_COOKIE}=")));
        assert!(!csrf.contains("HttpOnly"), "{csrf}");
        assert!(!csrf.contains(fresh.expose()) && !csrf.contains(token));

        let cookie = session.split("; ").next().unwrap();
        let csrf_value = csrf.split("; ").next().unwrap().split_once('=').unwrap().1;
        let reply = app
            .send(
                Method::POST,
                "/admin/api/v1/probe/admin",
                Some(cookie),
                Some(csrf_value),
                Some(json!({})),
            )
            .await;
        // A user-role session: CSRF and session pass, the role does not.
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(reply.code(), "forbidden");
    })
    .await;
}
