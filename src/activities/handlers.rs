use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

macro_rules! handler_get {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": {} })))
        }
    };
}
handler_get!(get_current); handler_get!(get_history); handler_get!(get_load);
handler_get!(get_zones); handler_get!(get_categories); handler_get!(get_transitions);
handler_get!(get_sedentary); handler_get!(get_exercise_log); handler_get!(get_recovery_status);

pub async fn manual_log(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Activity logged" } })))
}
