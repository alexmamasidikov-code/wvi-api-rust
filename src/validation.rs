// Centralized request-body validation.
//
// `ValidatedJson<T>` extracts JSON, runs `validator::Validate`, and on failure
// returns 422 Unprocessable Entity with a compact field→message map:
//
//   { "error": "validation_failed", "fields": { "rmssd": "must be 5.0-200.0" } }
//
// Per-metric range checks match physiological bounds + bracelet sensor specs.

use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use std::collections::BTreeMap;
use validator::{Validate, ValidationErrors};

/// Extractor that deserializes JSON then runs `Validate`.
/// Returns 400 on malformed JSON, 422 on validation failure.
pub struct ValidatedJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidatedJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(value) = Json::<T>::from_request(req, state)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({
                "error": "invalid_json",
                "message": e.body_text(),
            }))).into_response())?;
        value.validate().map_err(validation_error_response)?;
        Ok(ValidatedJson(value))
    }
}

/// Flatten `ValidationErrors` into `{ field: "first message" }`.
fn validation_error_response(errs: ValidationErrors) -> Response {
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    for (field, kinds) in errs.field_errors() {
        if let Some(first) = kinds.first() {
            let msg = first
                .message
                .as_ref()
                .map(|m| m.to_string())
                .unwrap_or_else(|| format!("invalid {}", first.code));
            fields.insert(field.to_string(), msg);
        }
    }
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": "validation_failed", "fields": fields })),
    )
        .into_response()
}
