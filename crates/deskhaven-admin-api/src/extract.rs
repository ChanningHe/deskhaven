//! Typed, validated input (06 §6): JSON bodies and query strings are deserialized into
//! request types and checked with `garde` before a handler runs. Every failure is an
//! enveloped error:
//!
//! | problem | status | code |
//! |---|---|---|
//! | body without `Content-Type: application/json` | 415 | `unsupported_media_type` |
//! | body that is not JSON | 400 | `bad_request` |
//! | JSON of the wrong shape (types, unknown fields with `deny_unknown_fields`) | 422 | `validation_failed` |
//! | query string of the wrong shape | 422 | `validation_failed` |
//! | a `garde` rule fails | 422 | `validation_failed`, `details.fields` |
//!
//! Body types should use `#[serde(deny_unknown_fields)]` (unknown fields are errors for
//! the admin API). `details` of a rule failure: `{"fields": {"<path>": ["<problem>", ...]}}`.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use garde::{Report, Validate};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::{AdminApiError, ErrorCode};

/// A validated JSON request body.
#[derive(Debug, Clone)]
pub struct ValidJson<T>(pub T);

/// A validated query string.
#[derive(Debug, Clone)]
pub struct ValidQuery<T>(pub T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()>,
{
    type Rejection = AdminApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(json_rejection)?;
        validated(value).map(Self)
    }
}

impl<S, T> FromRequestParts<S> for ValidQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate<Context = ()>,
{
    type Rejection = AdminApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(value) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(query_rejection)?;
        validated(value).map(Self)
    }
}

fn validated<T: Validate<Context = ()>>(value: T) -> Result<T, AdminApiError> {
    value.validate().map_err(report_error)?;
    Ok(value)
}

fn report_error(report: Report) -> AdminApiError {
    let mut fields: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (path, error) in report.iter() {
        fields
            .entry(path.to_string())
            .or_default()
            .push(error.message().to_owned());
    }
    AdminApiError::validation_failed("invalid input").with_details(json!({ "fields": fields }))
}

/// Longest `details.reason` returned (characters); serde messages can quote input.
pub const MAX_REASON_CHARS: usize = 256;

/// The rejection's message for the requester, bounded. Never logged: it may echo input.
fn reason(body_text: &str) -> String {
    body_text.chars().take(MAX_REASON_CHARS).collect()
}

fn json_rejection(rejection: JsonRejection) -> AdminApiError {
    let reason = reason(&rejection.body_text());
    let kind = rejection_kind(&rejection);
    let error = match rejection {
        JsonRejection::MissingJsonContentType(_) => AdminApiError::new(
            ErrorCode::UnsupportedMediaType,
            "expected Content-Type: application/json",
        ),
        JsonRejection::JsonSyntaxError(_) => {
            AdminApiError::new(ErrorCode::BadRequest, "malformed JSON body")
        }
        JsonRejection::JsonDataError(_) => AdminApiError::validation_failed("invalid request body")
            .with_details(json!({ "reason": reason })),
        other if other.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            AdminApiError::new(ErrorCode::PayloadTooLarge, "payload too large")
        }
        _ => AdminApiError::new(ErrorCode::BadRequest, "unreadable request body"),
    };
    tracing::debug!(kind, "request body rejected");
    error
}

fn rejection_kind(rejection: &JsonRejection) -> &'static str {
    match rejection {
        JsonRejection::MissingJsonContentType(_) => "content_type",
        JsonRejection::JsonSyntaxError(_) => "syntax",
        JsonRejection::JsonDataError(_) => "data",
        _ => "body",
    }
}

fn query_rejection(rejection: QueryRejection) -> AdminApiError {
    tracing::debug!("query string rejected");
    AdminApiError::validation_failed("invalid query string")
        .with_details(json!({ "reason": reason(&rejection.body_text()) }))
}
