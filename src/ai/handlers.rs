use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

macro_rules! ai_post {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": { "message": "AI processing" } })))
        }
    };
}
ai_post!(interpret); ai_post!(recommendations); ai_post!(chat);
ai_post!(explain_metric); ai_post!(action_plan); ai_post!(insights); ai_post!(genius_layer);
