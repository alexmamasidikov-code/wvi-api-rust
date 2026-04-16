use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::error::AppResult;

pub async fn server_status(State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let db_ok = sqlx::query("SELECT 1").execute(&pool).await.is_ok();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "status": "healthy",
            "database": if db_ok { "connected" } else { "disconnected" },
            "poolSize": pool.size(),
            "poolIdleConnections": pool.num_idle(),
            "cacheEnabled": true,
            "endpoints": 119,
            "version": "1.0.0",
            "uptime": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        }
    })))
}
pub async fn api_version() -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "version": "1.0.0", "engine": "WVI Rust/Axum" }
    })))
}
pub async fn docs_json() -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "openapi": "3.1.0",
        "info": { "title": "WVI API", "version": "1.0.0" }
    })))
}
