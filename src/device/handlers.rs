use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

macro_rules! h {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": {} })))
        }
    };
}
macro_rules! hp {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>, Json(_b): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": {} })))
        }
    };
}
h!(status); h!(capabilities); h!(firmware);
hp!(auto_monitoring); hp!(sync); hp!(measure);
