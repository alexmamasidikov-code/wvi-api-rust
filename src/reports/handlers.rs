use axum::{extract::{Path, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn generate(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "reportId": uuid::Uuid::new_v4() } })))
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
