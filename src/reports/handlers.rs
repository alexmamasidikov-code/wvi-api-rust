use axum::{extract::{Path, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn generate(user: AuthUser, State(pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    // 7-day aggregates
    let avg_hr = sqlx::query_scalar::<_, f64>("SELECT COALESCE(AVG(bpm)::float8, 0) FROM heart_rate WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'")
        .bind(uid).fetch_one(&pool).await.unwrap_or(0.0);
    let avg_hrv = sqlx::query_scalar::<_, f64>("SELECT COALESCE(AVG(rmssd)::float8, 0) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'")
        .bind(uid).fetch_one(&pool).await.unwrap_or(0.0);
    let avg_spo2 = sqlx::query_scalar::<_, f64>("SELECT COALESCE(AVG(value)::float8, 0) FROM spo2 WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'")
        .bind(uid).fetch_one(&pool).await.unwrap_or(0.0);
    let total_steps = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(steps)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'")
        .bind(uid).fetch_one(&pool).await.unwrap_or(0);
    let wvi_scores = sqlx::query_as::<_, (f32,)>("SELECT score FROM wvi_scores WHERE user_id = $1 AND calculated_at >= NOW() - INTERVAL '7 days' ORDER BY calculated_at")
        .bind(uid).fetch_all(&pool).await.unwrap_or_default();

    let avg_wvi = if !wvi_scores.is_empty() {
        wvi_scores.iter().map(|w| w.0 as f64).sum::<f64>() / wvi_scores.len() as f64
    } else {
        0.0
    };
    let best_wvi = wvi_scores.iter().map(|w| w.0).reduce(f32::max).unwrap_or(0.0);

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "period": "7 days",
            "generatedAt": chrono::Utc::now(),
            "summary": {
                "avgWVI": (avg_wvi * 10.0).round() / 10.0,
                "bestWVI": best_wvi,
                "avgHR": (avg_hr * 10.0).round() / 10.0,
                "avgHRV": (avg_hrv * 10.0).round() / 10.0,
                "avgSpO2": (avg_spo2 * 10.0).round() / 10.0,
                "totalSteps": total_steps,
                "dataPoints": wvi_scores.len(),
            }
        }
    })))
}

pub async fn list(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}

pub async fn get_templates(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}

pub async fn get_by_id(_user: AuthUser, State(_pool): State<PgPool>, Path(_id): Path<String>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}

pub async fn download(_user: AuthUser, State(_pool): State<PgPool>, Path(_id): Path<String>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}
