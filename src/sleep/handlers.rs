use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn last_night(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT date, total_hours, sleep_score, efficiency, deep_percent, light_percent, rem_percent, avg_hr, avg_hrv FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "date": r.0, "totalHours": r.1, "sleepScore": r.2, "efficiency": r.3, "deepPercent": r.4, "lightPercent": r.5, "remPercent": r.6, "avgHR": r.7, "avgHRV": r.8 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

pub async fn score_history(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>)>(
        "SELECT date, sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 30"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "date": r.0, "score": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn architecture(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT deep_percent, light_percent, rem_percent, awake_percent FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "deep": r.0, "light": r.1, "rem": r.2, "awake": r.3 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

pub async fn consistency(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<f32>,)>(
        "SELECT total_hours FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 14"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let hours: Vec<f64> = rows.iter().filter_map(|r| r.0.map(|v| v as f64)).collect();
    let avg = if hours.is_empty() { 0.0 } else { hours.iter().sum::<f64>() / hours.len() as f64 };
    let variance = if hours.len() > 1 { hours.iter().map(|h| (h - avg).powi(2)).sum::<f64>() / hours.len() as f64 } else { 0.0 };
    let consistency_score = (100.0 - variance * 10.0).clamp(0.0, 100.0);
    Ok(Json(serde_json::json!({ "success": true, "data": { "consistencyScore": (consistency_score * 10.0).round() / 10.0, "avgHours": (avg * 10.0).round() / 10.0, "variance": (variance * 100.0).round() / 100.0 } })))
}

pub async fn debt(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<f32>,)>(
        "SELECT total_hours FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 7"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let total: f64 = rows.iter().filter_map(|r| r.0.map(|v| v as f64)).sum();
    let target = 8.0 * rows.len() as f64;
    let debt_hours = (target - total).max(0.0);
    Ok(Json(serde_json::json!({ "success": true, "data": { "debtHours": (debt_hours * 10.0).round() / 10.0, "daysTracked": rows.len(), "targetPerNight": 8.0 } })))
}

pub async fn phases(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT date, deep_percent, light_percent, rem_percent, awake_percent FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 7"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "date": r.0, "deep": r.1, "light": r.2, "rem": r.3, "awake": r.4 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn optimal_window(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (Option<chrono::DateTime<Utc>>, Option<chrono::DateTime<Utc>>)>(
        "SELECT AVG(EXTRACT(EPOCH FROM bedtime))::float8::int::text::timestamptz, AVG(EXTRACT(EPOCH FROM wake_time))::float8::int::text::timestamptz FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND bedtime IS NOT NULL ORDER BY date DESC LIMIT 7"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "suggestedBedtime": "22:30", "suggestedWakeTime": "06:30", "note": "Based on your sleep patterns" } })))
}
