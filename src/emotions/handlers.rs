use axum::{extract::State, Json};
use chrono::{Utc, Duration};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// GET /emotions/current
pub async fn get_current(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (String, Option<f32>, Option<String>, Option<f32>, Option<serde_json::Value>, chrono::DateTime<Utc>)>(
        "SELECT primary_emotion, primary_confidence, secondary_emotion, secondary_confidence, all_scores, timestamp FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "primary": r.0, "primaryConfidence": r.1, "secondary": r.2, "secondaryConfidence": r.3, "allScores": r.4, "timestamp": r.5 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

/// GET /emotions/history
pub async fn get_history(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (String, Option<f32>, chrono::DateTime<Utc>)>(
        "SELECT primary_emotion, primary_confidence, timestamp FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "emotion": r.0, "confidence": r.1, "timestamp": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /emotions/wellbeing — average emotional wellbeing score (24h)
pub async fn get_wellbeing(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let positive = sqlx::query_as::<_, (i64,)>(
        "SELECT COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '24 hours' AND primary_emotion IN ('calm','relaxed','joyful','energized','excited','focused','meditative','flow')"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    let total = sqlx::query_as::<_, (i64,)>(
        "SELECT COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '24 hours'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    let score = if total > 0 { (positive as f64 / total as f64 * 100.0 * 10.0).round() / 10.0 } else { 50.0 };
    Ok(Json(serde_json::json!({ "success": true, "data": { "score": score, "positiveCount": positive, "totalCount": total } })))
}

/// GET /emotions/distribution
pub async fn get_distribution(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT primary_emotion, COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' GROUP BY primary_emotion ORDER BY count DESC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "emotion": r.0, "count": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /emotions/heatmap — emotion by hour of day
pub async fn get_heatmap(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (f64, String, i64)>(
        "SELECT EXTRACT(HOUR FROM timestamp)::float8, primary_emotion, COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' GROUP BY 1, 2 ORDER BY 1, 3 DESC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "hour": r.0 as u32, "emotion": r.1, "count": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /emotions/transitions
pub async fn get_transitions(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT a.primary_emotion as from_emotion, b.primary_emotion as to_emotion, COUNT(*) FROM emotions a JOIN emotions b ON a.user_id = b.user_id AND b.timestamp = (SELECT MIN(c.timestamp) FROM emotions c WHERE c.user_id = a.user_id AND c.timestamp > a.timestamp) WHERE a.user_id = (SELECT id FROM users WHERE privy_did = $1) AND a.timestamp >= NOW() - INTERVAL '7 days' AND a.primary_emotion != b.primary_emotion GROUP BY 1, 2 ORDER BY 3 DESC LIMIT 20"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "from": r.0, "to": r.1, "count": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /emotions/triggers
pub async fn get_triggers(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "note": "Triggers analysis requires correlated biometric + emotion data" } })))
}

/// GET /emotions/streaks
pub async fn get_streaks(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let positive_streak = sqlx::query_as::<_, (i64,)>(
        "WITH ranked AS (SELECT primary_emotion, timestamp, ROW_NUMBER() OVER (ORDER BY timestamp DESC) as rn FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 100) SELECT COUNT(*) FROM ranked WHERE primary_emotion IN ('calm','relaxed','joyful','energized','excited','focused','meditative','flow') AND rn <= (SELECT MIN(rn) FROM ranked WHERE primary_emotion NOT IN ('calm','relaxed','joyful','energized','excited','focused','meditative','flow'))"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    Ok(Json(serde_json::json!({ "success": true, "data": { "currentPositiveStreak": positive_streak } })))
}
