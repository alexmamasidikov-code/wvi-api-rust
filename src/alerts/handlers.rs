use axum::{extract::{Path, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

macro_rules! h {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": [] })))
        }
    };
}
h!(list); h!(active); h!(get_settings); h!(get_history); h!(stats);
pub async fn acknowledge(_user: AuthUser, State(_pool): State<PgPool>, Path(_id): Path<String>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "acknowledged": true } })))
}
