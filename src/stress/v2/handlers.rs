//! Stress v2 HTTP handlers.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::State, Json};
use sqlx::PgPool;

pub async fn get_intraday(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let rows: Vec<(chrono::DateTime<chrono::Utc>, f64, String, bool)> = sqlx::query_as(
        "SELECT ts, score, level, micro_pulse FROM stress_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;
    let points: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(ts, s, l, p)| {
            serde_json::json!({"ts": ts, "score": s, "level": l, "micro_pulse": p})
        })
        .collect();
    Ok(Json(serde_json::json!({"points": points})))
}
