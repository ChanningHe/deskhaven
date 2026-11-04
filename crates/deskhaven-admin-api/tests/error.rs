//! `AdminApiError` -> status + `{error:{code,message,details?}}` (08-web-console §6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::io;

use axum::body::to_bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use deskhaven_admin_api::{AdminApiError, ErrorCode};
use serde_json::{Value, json};

async fn json_of(res: Response) -> (StatusCode, Value) {
    let status = res.status();
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let body = to_bytes(res.into_body(), 4096).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

#[test]
fn codes_are_stable_snake_case_with_fixed_statuses() {
    let cases = [
        (
            ErrorCode::ValidationFailed,
            "validation_failed",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ErrorCode::Unauthorized,
            "unauthorized",
            StatusCode::UNAUTHORIZED,
        ),
        (ErrorCode::Forbidden, "forbidden", StatusCode::FORBIDDEN),
        (ErrorCode::NotFound, "not_found", StatusCode::NOT_FOUND),
        (ErrorCode::Conflict, "conflict", StatusCode::CONFLICT),
        (
            ErrorCode::Internal,
            "internal",
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        // M3-02: none of the codes a session holder can trigger is a 401.
        (
            ErrorCode::InvalidCredentials,
            "invalid_credentials",
            StatusCode::UNAUTHORIZED,
        ),
        (
            ErrorCode::InvalidChallenge,
            "invalid_challenge",
            StatusCode::UNAUTHORIZED,
        ),
        (
            ErrorCode::InvalidCode,
            "invalid_code",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ErrorCode::WrongPassword,
            "wrong_password",
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            ErrorCode::AccountDisabled,
            "account_disabled",
            StatusCode::FORBIDDEN,
        ),
        (
            ErrorCode::AccountUnverified,
            "account_unverified",
            StatusCode::FORBIDDEN,
        ),
        (
            ErrorCode::InsecureTransport,
            "insecure_transport",
            StatusCode::FORBIDDEN,
        ),
        (
            ErrorCode::SetupRequired,
            "setup_required",
            StatusCode::FORBIDDEN,
        ),
        (
            ErrorCode::InvalidSetupToken,
            "invalid_setup_token",
            StatusCode::FORBIDDEN,
        ),
        (
            ErrorCode::SetupCompleted,
            "setup_completed",
            StatusCode::CONFLICT,
        ),
        (ErrorCode::OidcFailed, "oidc_failed", StatusCode::FORBIDDEN),
        (
            ErrorCode::NotLocalAccount,
            "not_local_account",
            StatusCode::CONFLICT,
        ),
        (ErrorCode::TfaEnforced, "tfa_enforced", StatusCode::CONFLICT),
    ];
    for (code, wire, status) in cases {
        assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
        assert_eq!(code.as_str(), wire);
        assert_eq!(code.status(), status);
    }
}

#[tokio::test]
async fn error_without_details_omits_the_field() {
    let got = json_of(AdminApiError::not_found("user not found").into_response()).await;
    assert_eq!(
        got,
        (
            StatusCode::NOT_FOUND,
            json!({ "error": { "code": "not_found", "message": "user not found" } })
        )
    );
}

#[tokio::test]
async fn validation_error_carries_details() {
    let details = json!({ "fields": { "email": ["invalid"] } });
    let err = AdminApiError::validation_failed("invalid input").with_details(details.clone());
    let got = json_of(err.into_response()).await;
    let expected = json!({
        "error": { "code": "validation_failed", "message": "invalid input", "details": details }
    });
    assert_eq!(got, (StatusCode::UNPROCESSABLE_ENTITY, expected));
}

#[tokio::test]
async fn convenience_constructors_use_matching_codes() {
    let cases = [
        (AdminApiError::unauthorized(), ErrorCode::Unauthorized),
        (AdminApiError::forbidden(), ErrorCode::Forbidden),
        (
            AdminApiError::conflict("version mismatch"),
            ErrorCode::Conflict,
        ),
        (
            AdminApiError::new(ErrorCode::NotFound, "gone"),
            ErrorCode::NotFound,
        ),
    ];
    for (err, code) in cases {
        assert_eq!(err.code(), code);
        let (status, body) = json_of(err.into_response()).await;
        assert_eq!(status, code.status());
        assert_eq!(body["error"]["code"], json!(code.as_str()));
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty())
        );
    }
}

#[tokio::test]
async fn internal_error_never_leaks_detail() {
    let err = AdminApiError::internal(io::Error::other("db password=hunter2"));
    let got = json_of(err.into_response()).await;
    assert_eq!(
        got,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": { "code": "internal", "message": "internal server error" } })
        )
    );
}
