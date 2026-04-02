use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn get_settings(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}
pub async fn update_settings(_user: AuthUser, State(_pool): State<PgPool>, Json(_b): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Settings updated" } })))
}
pub async fn get_notifications(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}
pub async fn update_notifications(_user: AuthUser, State(_pool): State<PgPool>, Json(_b): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Notifications updated" } })))
}
