use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn get_me(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "userId": _user.user_id, "email": _user.email } })))
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
