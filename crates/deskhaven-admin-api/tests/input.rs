//! Wire conventions (08 §6): enveloped input errors, garde validation, pagination shape.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use common::{PEER, harness, probe};
use deskhaven_admin_api::CSRF_HEADER;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

const SELF_WRITE: &str = "/admin/api/v1/probe/self";

#[tokio::test]
async fn valid_body_reaches_the_handler() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;

        let reply = app
            .write(
                Method::POST,
                SELF_WRITE,
                &uma,
                json!({ "name": "box", "count": 3 }),
            )
            .await;

        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
        assert_eq!(reply.body, json!({ "name": "box", "count": 3 }));
    })
    .await;
}

#[tokio::test]
async fn rule_failures_are_422_with_field_details() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;

        let reply = app
            .write(
                Method::POST,
                SELF_WRITE,
                &uma,
                json!({ "name": "", "count": 11 }),
            )
            .await;

        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.code(), "validation_failed");
        let fields = &reply.body["error"]["details"]["fields"];
        assert_eq!(fields.as_object().unwrap().len(), 2, "{fields}");
        assert!(fields["name"][0].is_string(), "{fields}");
        assert!(fields["count"][0].is_string(), "{fields}");
    })
    .await;
}

#[tokio::test]
async fn unknown_fields_and_wrong_types_are_422() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;

        for body in [
            json!({ "name": "box", "count": 3, "extra": true }),
            json!({ "name": "box", "count": "three" }),
            json!({ "name": "box" }),
        ] {
            let reply = app
                .write(Method::POST, SELF_WRITE, &uma, body.clone())
                .await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(reply.code(), "validation_failed");
            assert!(reply.body["error"]["details"]["reason"].is_string());
        }
    })
    .await;
}

#[tokio::test]
async fn malformed_json_is_400_and_non_json_is_415() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;
        let request = |content_type: &str, body: &'static str| {
            Request::post(SELF_WRITE)
                .extension(ConnectInfo(PEER))
                .header(header::COOKIE, &uma.cookie)
                .header(CSRF_HEADER, &uma.csrf)
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(body))
                .unwrap()
        };

        let malformed = app.raw(request("application/json", "{not json")).await;
        let form = app
            .raw(request("text/plain", r#"{"name":"a","count":1}"#))
            .await;

        assert_eq!(
            (malformed.status, malformed.code()),
            (StatusCode::BAD_REQUEST, "bad_request")
        );
        assert_eq!(
            (form.status, form.code()),
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
        );
    })
    .await;
}

#[tokio::test]
async fn pagination_has_defaults_bounds_and_the_list_shape() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;

        let first = app.get("/admin/api/v1/probe/list", &uma).await;
        let last = app
            .get("/admin/api/v1/probe/list?page=3&page_size=20", &uma)
            .await;

        assert_eq!(first.status, StatusCode::OK);
        assert_eq!(first.body["total"], 45);
        assert_eq!(
            (first.body["page"].clone(), first.body["page_size"].clone()),
            (json!(1), json!(20))
        );
        assert_eq!(first.body["items"].as_array().unwrap().len(), 20);
        assert_eq!(
            last.body,
            json!({ "items": [40, 41, 42, 43, 44], "total": 45, "page": 3, "page_size": 20 })
        );
        for query in ["page_size=201", "page_size=0", "page=0", "page=abc"] {
            let reply = app
                .get(&format!("/admin/api/v1/probe/list?{query}"), &uma)
                .await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
            assert_eq!(reply.code(), "validation_failed");
        }
        let too_big = app
            .get("/admin/api/v1/probe/list?page_size=201", &uma)
            .await;
        assert!(too_big.body["error"]["details"]["fields"]["page_size"][0].is_string());
        let max = app
            .get("/admin/api/v1/probe/list?page_size=200", &uma)
            .await;
        assert_eq!(max.status, StatusCode::OK);
    })
    .await;
}

#[tokio::test]
async fn rejection_reason_is_bounded() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;
        let uma = app.console("uma", Role::User).await;
        let long_field = "x".repeat(2_000);

        let reply = app
            .write(
                Method::POST,
                SELF_WRITE,
                &uma,
                json!({ "name": "box", "count": 1, long_field: true }),
            )
            .await;

        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        let reason = reply.body["error"]["details"]["reason"].as_str().unwrap();
        assert!(
            reason.chars().count() <= deskhaven_admin_api::MAX_REASON_CHARS,
            "{reason}"
        );
    })
    .await;
}

#[tokio::test]
async fn unknown_admin_paths_are_enveloped_404() {
    each_migrated_backend(async |db: &TestDb| {
        let app = harness(db, probe::routes()).await;

        let reply = app
            .send(Method::GET, "/admin/api/v1/nope", None, None, None)
            .await;

        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(
            reply.body,
            json!({ "error": { "code": "not_found", "message": "not found" } })
        );
    })
    .await;
}
