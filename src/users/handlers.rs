use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};

pub async fn get_me(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "userId": _user.privy_did, "email": _user.email } })))
}
pub async fn update_me(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Profile updated" } })))
}
pub async fn get_norms(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "restingHR": 65, "baseTemp": 36.6 } })))
}
pub async fn calibrate(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Calibration started" } })))
}

// MARK: — Persona (HOME layout selector, Sprint 3 of NPS uplift)

#[derive(Serialize)]
pub struct PersonaResponse {
    pub persona: Option<String>,
}

#[derive(Deserialize)]
pub struct PersonaUpdate {
    pub persona: String,
}

const ALLOWED_PERSONAS: &[&str] = &["athlete", "professional", "parent", "curious"];

/// `GET /api/v1/users/me/persona`
pub async fn get_persona(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<PersonaResponse>> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT persona FROM users WHERE privy_did = $1",
    )
    .bind(&user.privy_did)
    .fetch_optional(&pool)
    .await?;

    Ok(Json(PersonaResponse {
        persona: row.and_then(|r| r.0),
    }))
}

/// `PUT /api/v1/users/me/persona`
pub async fn put_persona(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<PersonaUpdate>,
) -> AppResult<Json<PersonaResponse>> {
    let value = body.persona.trim().to_lowercase();
    if !ALLOWED_PERSONAS.contains(&value.as_str()) {
        return Err(AppError::BadRequest(format!(
            "persona must be one of {:?}",
            ALLOWED_PERSONAS
        )));
    }

    sqlx::query("UPDATE users SET persona = $1, updated_at = NOW() WHERE privy_did = $2")
        .bind(&value)
        .bind(&user.privy_did)
        .execute(&pool)
        .await?;

    Ok(Json(PersonaResponse {
        persona: Some(value),
    }))
}
