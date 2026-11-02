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

#[utoipa::path(get, path = "/users/{id}", security(("console_session" = ["admin"])), params(("id" = i64, Path)), responses((status = 200)))]
async fn user_by_id() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(delete, path = "/users/{user_id}", security(("console_session" = ["admin"])), params(("user_id" = i64, Path)), responses((status = 200)))]
async fn user_by_other_name() -> Json<Value> {
    Json(json!({}))
}

#[utoipa::path(post, path = "/login", security(()), responses((status = 200)))]
async fn public_post() -> Json<Value> {
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

#[test]
fn differently_named_path_parameters_fail_route_assembly() {
    let error = err(AdminRoutes::new()
        .routes(routes!(user_by_id))
        .and_then(|r| r.routes(routes!(user_by_other_name))));

    assert_eq!(error.path, "/admin/api/v1/users/{user_id}");
    assert_eq!(error.reason, "path parameters named differently");
}

#[test]
fn write_operations_document_the_csrf_header() {
    let routes = AdminRoutes::new()
        .routes(routes!(mixed_read))
        .and_then(|r| r.routes(routes!(mixed_write)))
        .and_then(|r| r.routes(routes!(public_post)))
        .unwrap();
    let doc = serde_json::to_value(routes.openapi()).unwrap();
    let csrf = json!({
        "name": "X-Deskhaven-CSRF",
        "in": "header",
        "required": true,
    });

    let read = &doc["paths"]["/admin/api/v1/mixed"]["get"];
    assert!(read.get("parameters").is_none(), "{read}");
    for op in [
        &doc["paths"]["/admin/api/v1/mixed"]["post"],
        &doc["paths"]["/admin/api/v1/login"]["post"],
    ] {
        let [param] = op["parameters"].as_array().unwrap().as_slice() else {
            panic!("{op}");
        };
        for key in ["name", "in", "required"] {
            assert_eq!(param[key], csrf[key], "{op}");
        }
        assert_eq!(
            op["responses"]["403"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/ErrorEnvelope"
        );
    }
    let public = &doc["paths"]["/admin/api/v1/login"]["post"]["responses"];
    assert!(public.get("401").is_none(), "{public}");
}
