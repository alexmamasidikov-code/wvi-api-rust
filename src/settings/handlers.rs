use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn get_settings(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (String, String, String, String, i32, serde_json::Value)>(
        "SELECT units, language, timezone, theme, data_retention, privacy FROM app_settings WHERE user_id = (SELECT id FROM users WHERE privy_did = $1)"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "units": r.0, "language": r.1, "timezone": r.2, "theme": r.3, "dataRetention": r.4, "privacy": r.5 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": { "units": "metric", "language": "en", "timezone": "UTC", "theme": "auto", "dataRetention": 365 } }))),
    }
}

pub async fn update_settings(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1").bind(&user.privy_did).fetch_optional(&pool).await?.ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))?;
    sqlx::query("INSERT INTO app_settings (user_id, units, language, timezone, theme, updated_at) VALUES ($1, $2, $3, $4, $5, NOW()) ON CONFLICT (user_id) DO UPDATE SET units = COALESCE($2, app_settings.units), language = COALESCE($3, app_settings.language), timezone = COALESCE($4, app_settings.timezone), theme = COALESCE($5, app_settings.theme), updated_at = NOW()")
        .bind(uid)
        .bind(body.get("units").and_then(|v| v.as_str()))
        .bind(body.get("language").and_then(|v| v.as_str()))
        .bind(body.get("timezone").and_then(|v| v.as_str()))
        .bind(body.get("theme").and_then(|v| v.as_str()))
        .execute(&pool).await?;
    crate::audit::log_action(&pool, &user.privy_did, "settings.update", "settings", None, Some(body.clone()), None, None).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Settings updated" } })))
}

pub async fn get_notifications(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (bool, bool, bool, serde_json::Value, serde_json::Value)>(
        "SELECT push, email, sms, quiet_hours, alert_levels FROM notification_settings WHERE user_id = (SELECT id FROM users WHERE privy_did = $1)"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "push": r.0, "email": r.1, "sms": r.2, "quietHours": r.3, "alertLevels": r.4 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": { "push": true, "email": false, "sms": false } }))),
    }
}

pub async fn update_notifications(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1").bind(&user.privy_did).fetch_optional(&pool).await?.ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))?;
    sqlx::query("INSERT INTO notification_settings (user_id, push, email, sms, updated_at) VALUES ($1, $2, $3, $4, NOW()) ON CONFLICT (user_id) DO UPDATE SET push = COALESCE($2, notification_settings.push), email = COALESCE($3, notification_settings.email), sms = COALESCE($4, notification_settings.sms), updated_at = NOW()")
        .bind(uid)
        .bind(body.get("push").and_then(|v| v.as_bool()))
        .bind(body.get("email").and_then(|v| v.as_bool()))
        .bind(body.get("sms").and_then(|v| v.as_bool()))
        .execute(&pool).await?;
    crate::audit::log_action(&pool, &user.privy_did, "settings.update_notifications", "notification_settings", None, Some(body.clone()), None, None).await;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Notifications updated" } })))
}
