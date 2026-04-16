use axum::{extract::State, Json};
use serde::Deserialize;
use sqlx::PgPool;

use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

#[derive(Deserialize)]
pub struct RegisterReq {
    pub token: String,
    #[serde(default = "default_env")]
    pub env: String, // "development" | "production"
}
fn default_env() -> String { "development".into() }

/// POST /api/v1/notifications/register
/// Idempotent — overwrites last_seen_at on repeat.
pub async fn register_token(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<RegisterReq>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE privy_did = $1")
        .bind(&user.privy_did)
        .fetch_one(&pool)
        .await
        .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO push_tokens (user_id, token, env, last_seen_at)
        VALUES ($1, $2, $3, NOW())
        ON CONFLICT (user_id, token) DO UPDATE SET last_seen_at = NOW(), env = EXCLUDED.env
        "#,
    )
    .bind(user_id)
    .bind(&body.token)
    .bind(&body.env)
    .execute(&pool)
    .await
    .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "success": true })))
}
