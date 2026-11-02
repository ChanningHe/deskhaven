//! Route assembly: every operation must declare a permission, or the route set (and so
//! server startup, which builds it) fails. Also the generated OpenAPI document.

#![allow(clippy::unwrap_used, clippy::panic)]

use axum::Json;
use deskhaven_admin_api::{ADMIN_API_V1, AdminRoutes, RouteError, api_routes, openapi_json};
use serde_json::{Value, json};
use utoipa_axum::routes;

#[utoipa::path(get, path = "/undeclared", responses((status = 200)))]
async fn undeclared() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(get, path = "/superuser", security(("console_session" = ["superuser"])), responses((status = 200)))]
async fn unknown_role() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(get, path = "/mixed", security(("console_session" = ["self"])), responses((status = 200)))]
async fn mixed_read() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(post, path = "/mixed", security(("console_session" = ["owner"])), responses((status = 200)))]
async fn mixed_write() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(get, path = "/me", security(("console_session" = ["admin"])), responses((status = 200)))]
async fn shadow_me() -> Json<Value> {
    Json(json!({}))
}

fn err(routes: Result<AdminRoutes, RouteError>) -> RouteError {
    match routes {
        Ok(_) => panic!("route set assembled"),
        Err(err) => err,
    }
}

#[test]
fn undeclared_permission_fails_route_assembly() {
    let error = err(AdminRoutes::new().routes(routes!(undeclared)));

    assert_eq!(error.method, "GET");
    assert_eq!(error.path, format!("{ADMIN_API_V1}/undeclared"));
    assert!(
        error.to_string().contains("no permission declared"),
        "{error}"
    );
}

#[test]
fn undeclared_permission_fails_the_production_route_set() {
    let error = err(api_routes()
        .unwrap()
        .merge(AdminRoutes::new())
        .and_then(|r| r.routes(routes!(undeclared))));

    assert_eq!(error.path, "/admin/api/v1/undeclared");
}

#[test]
fn unknown_role_fails_route_assembly() {
    let error = err(AdminRoutes::new().routes(routes!(unknown_role)));

    assert!(error.reason.contains("unknown role"), "{error}");
}

#[test]
fn different_permissions_in_one_routes_call_fail() {
    let error = err(AdminRoutes::new().routes(routes!(mixed_read, mixed_write)));

    assert!(error.reason.contains("permissions differ"), "{error}");
    let split = AdminRoutes::new()
        .routes(routes!(mixed_read))
        .and_then(|r| r.routes(routes!(mixed_write)));
    assert!(split.is_ok());
}

#[test]
fn an_operation_declared_twice_fails_instead_of_panicking() {
    let error = err(api_routes().unwrap().routes(routes!(shadow_me)));

    assert_eq!(
        (error.method.as_str(), error.reason),
        ("GET", "operation declared twice")
    );
}

#[test]
fn production_document_declares_auth_on_every_operation() {
    let doc: Value = serde_json::from_str(&openapi_json().unwrap()).unwrap();

    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(
        doc["components"]["securitySchemes"]["console_session"],
        json!({
            "type": "apiKey",
            "in": "cookie",
            "name": "__Host-deskhaven_session",
            "description": "Console session token (HttpOnly; set by the console login)."
        })
    );
    let me = &doc["paths"]["/admin/api/v1/me"]["get"];
    assert_eq!(me["security"], json!([{ "console_session": ["self"] }]));
    for status in ["401", "403", "429"] {
        assert_eq!(
            me["responses"][status]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ErrorEnvelope",
            "{status}"
        );
    }
    assert!(doc["components"]["schemas"]["ErrorEnvelope"].is_object());
    assert!(doc["components"]["schemas"]["Me"].is_object());
    for (path, item) in doc["paths"].as_object().unwrap() {
        for (method, op) in item.as_object().unwrap() {
            assert!(op["security"].is_array(), "{method} {path} has no security");
        }
    }
}

#[test]
fn document_is_deterministic() {
    assert_eq!(openapi_json().unwrap(), openapi_json().unwrap());
}
