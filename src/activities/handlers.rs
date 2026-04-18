use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn get_current(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>, Option<String>)>(
        "SELECT timestamp, steps, calories, mets, activity_type FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "timestamp": r.0, "steps": r.1, "calories": r.2, "mets": r.3, "activityType": r.4 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

pub async fn get_history(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<String>)>(
        "SELECT timestamp, steps, calories, active_minutes, mets, activity_type FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "steps": r.1, "calories": r.2, "activeMinutes": r.3, "mets": r.4, "activityType": r.5 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_load(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let today = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "SELECT SUM(active_minutes)::float8, SUM(calories)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= CURRENT_DATE"
    ).bind(&user.privy_did).fetch_one(&pool).await?;
    let week = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "SELECT SUM(active_minutes)::float8, SUM(calories)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "today": { "activeMinutes": today.0, "calories": today.1 }, "week": { "activeMinutes": week.0, "calories": week.1 } } })))
}

pub async fn get_zones(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "note": "HR zone distribution requires continuous HR data with zone classification" } })))
}

pub async fn get_categories(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<String>, i64, Option<f64>)>(
        "SELECT activity_type, COUNT(*), SUM(active_minutes)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '30 days' AND activity_type IS NOT NULL GROUP BY activity_type ORDER BY count DESC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "type": r.0, "count": r.1, "totalMinutes": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_transitions(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}

pub async fn get_sedentary(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    // Device reports cumulative counters; clamp to a day window (0..1440) to avoid leaking lifetime totals.
    let row = sqlx::query_as::<_, (Option<f64>, Option<i64>)>(
        "SELECT LEAST(GREATEST(MAX(active_minutes)::float8 - MIN(active_minutes)::float8, 0.0), 1440.0), COUNT(*) \
         FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) \
         AND timestamp >= CURRENT_DATE AND timestamp < CURRENT_DATE + INTERVAL '1 day'"
    ).bind(&user.privy_did).fetch_one(&pool).await?;
    let active_today = row.0.unwrap_or(0.0);
    let samples = row.1.unwrap_or(0);
    let sedentary = (16.0 * 60.0 - active_today).max(0.0).min(16.0 * 60.0);
    let active_breaks = (samples / 2).max(0);
    let longest_sit_min = sedentary.round() as i64;
    Ok(Json(serde_json::json!({ "success": true, "data": {
        "sedentary_minutes": sedentary.round() as i64,
        "active_breaks": active_breaks,
        "longest_sit_min": longest_sit_min,
        "sedentaryMinutes": sedentary,
        "activeMinutes": active_today,
        "ratio": if active_today > 0.0 { sedentary / active_today } else { 0.0 }
    } })))
}

pub async fn get_exercise_log(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<String>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT timestamp, activity_type, active_minutes, calories, mets FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND activity_type IS NOT NULL AND timestamp >= NOW() - INTERVAL '30 days' ORDER BY timestamp DESC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "type": r.1, "minutes": r.2, "calories": r.3, "mets": r.4 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_recovery_status(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let hrv = sqlx::query_as::<_, (Option<f32>,)>("SELECT rmssd FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?.and_then(|r| r.0).unwrap_or(50.0);
    let sleep_score = sqlx::query_as::<_, (Option<f32>,)>("SELECT sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?.and_then(|r| r.0).unwrap_or(50.0);
    let recovery = (hrv / 100.0 * 50.0 + sleep_score / 100.0 * 50.0).min(100.0);
    let status = if recovery > 75.0 { "ready" } else if recovery > 50.0 { "moderate" } else { "fatigued" };
    Ok(Json(serde_json::json!({ "success": true, "data": { "recoveryScore": recovery, "status": status, "hrv": hrv, "sleepScore": sleep_score } })))
}

pub async fn manual_log(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1").bind(&user.privy_did).fetch_optional(&pool).await?.ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))?;
    sqlx::query("INSERT INTO activity (user_id, timestamp, steps, calories, active_minutes, mets, activity_type) VALUES ($1, NOW(), $2, $3, $4, $5, $6)")
        .bind(uid)
        .bind(body.get("steps").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("calories").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("activeMinutes").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("mets").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("activityType").and_then(|v| v.as_str()))
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Activity logged" } })))
}
