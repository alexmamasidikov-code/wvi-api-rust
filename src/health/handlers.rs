use axum::Json;
use crate::error::AppResult;

pub async fn server_status() -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "status": "healthy",
            "uptime": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            "version": env!("CARGO_PKG_VERSION"),
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
