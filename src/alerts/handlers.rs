use axum::{extract::{Path, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn list(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, String, Option<f32>, chrono::DateTime<chrono::Utc>, bool)>(
        "SELECT id, level, metric, message, value, created_at, acknowledged FROM alerts WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY created_at DESC LIMIT 100"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "id": r.0, "level": r.1, "metric": r.2, "message": r.3, "value": r.4, "createdAt": r.5, "acknowledged": r.6 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn active(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, String, chrono::DateTime<chrono::Utc>)>(
        "SELECT id, level, metric, message, created_at FROM alerts WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND acknowledged = false ORDER BY created_at DESC"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "id": r.0, "level": r.1, "metric": r.2, "message": r.3, "createdAt": r.4 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_settings(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (bool, serde_json::Value, serde_json::Value)>(
        "SELECT enabled, thresholds, channels FROM alert_settings WHERE user_id = (SELECT id FROM users WHERE privy_did = $1)"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "enabled": r.0, "thresholds": r.1, "channels": r.2 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": { "enabled": true, "thresholds": {}, "channels": { "push": true } } }))),
    }
}

pub async fn get_history(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, chrono::DateTime<chrono::Utc>, bool)>(
        "SELECT id, level, message, created_at, acknowledged FROM alerts WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND acknowledged = true ORDER BY created_at DESC LIMIT 50"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "id": r.0, "level": r.1, "message": r.2, "createdAt": r.3 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn acknowledge(user: AuthUser, State(pool): State<PgPool>, Path(id): Path<String>) -> AppResult<Json<serde_json::Value>> {
    sqlx::query("UPDATE alerts SET acknowledged = true, acknowledged_at = NOW() WHERE id = $1::uuid AND user_id = (SELECT id FROM users WHERE privy_did = $2)")
        .bind(&id).bind(&user.privy_did).execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "acknowledged": true } })))
}

pub async fn stats(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT level, COUNT(*) FROM alerts WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND created_at >= NOW() - INTERVAL '30 days' GROUP BY level"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "level": r.0, "count": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}
