use axum::{extract::{Query, State}, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

macro_rules! bio_get {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": [] })))
        }
    };
}
macro_rules! bio_post {
    ($name:ident) => {
        pub async fn $name(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
            Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 0 } })))
        }
    };
}

bio_get!(get_heart_rate); bio_post!(post_heart_rate);
bio_get!(get_hrv); bio_post!(post_hrv);
bio_get!(get_spo2); bio_post!(post_spo2);
bio_get!(get_temperature); bio_post!(post_temperature);
bio_get!(get_sleep); bio_post!(post_sleep);
bio_get!(get_ppi); bio_post!(post_ppi);
bio_get!(get_ecg); bio_post!(post_ecg);
bio_get!(get_activity); bio_post!(post_activity);
bio_get!(get_blood_pressure);
bio_get!(get_stress);
bio_get!(get_breathing_rate);
bio_get!(get_rmssd);
bio_get!(get_coherence);
bio_get!(get_realtime);
bio_get!(get_summary);

pub async fn sync(_user: AuthUser, State(_pool): State<PgPool>, Json(_body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "syncId": uuid::Uuid::new_v4(), "recordsProcessed": 0 } })))
}
