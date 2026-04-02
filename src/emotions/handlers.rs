use axum::{extract::{Query, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use super::models::*;

pub async fn get_current(
    _user: AuthUser,
    State(_pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    // In production: fetch latest biometrics and run emotion engine
    let result = EmotionResult {
        primary: EmotionState::Calm,
        primary_confidence: 0.72,
        secondary: EmotionState::Focused,
        secondary_confidence: 0.45,
        emoji: "😌".into(),
        category: "positive".into(),
        label: "Спокойствие".into(),
        all_scores: vec![],
        timestamp: chrono::Utc::now(),
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": result,
    })))
}

pub async fn get_history(
    _user: AuthUser,
    State(_pool): State<PgPool>,
    Query(q): Query<crate::wvi::models::WVIHistoryQuery>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "success": true,
        "data": [],
    })))
}

pub async fn get_wellbeing(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "score": 65.0, "trend": "stable" } })))
}

pub async fn get_distribution(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}

pub async fn get_heatmap(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": {} })))
}

pub async fn get_transitions(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}

pub async fn get_triggers(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}

pub async fn get_streaks(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": [] })))
}
