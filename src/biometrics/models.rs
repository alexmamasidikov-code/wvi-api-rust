use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct HeartRateRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub bpm: f32,
    pub confidence: Option<f32>,
    pub zone: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct HRVRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub sdnn: Option<f32>,
    pub rmssd: Option<f32>,
    pub pnn50: Option<f32>,
    pub ln_rmssd: Option<f32>,
    pub stress: Option<f32>,
    pub heart_rate: Option<f32>,
    pub systolic_bp: Option<f32>,
    pub diastolic_bp: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SpO2Record {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub value: f32,
    pub confidence: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TemperatureRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub value: f32,
    pub location: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SleepRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub date: chrono::NaiveDate,
    pub bedtime: Option<DateTime<Utc>>,
    pub wake_time: Option<DateTime<Utc>>,
    pub total_hours: Option<f32>,
    pub sleep_score: Option<f32>,
    pub efficiency: Option<f32>,
    pub deep_percent: Option<f32>,
    pub light_percent: Option<f32>,
    pub rem_percent: Option<f32>,
    pub awake_percent: Option<f32>,
    pub avg_hr: Option<f32>,
    pub avg_hrv: Option<f32>,
    pub avg_spo2: Option<f32>,
    pub respiratory_rate: Option<f32>,
    pub disturbances: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PPIRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub intervals: Option<serde_json::Value>,
    pub rmssd: Option<f32>,
    pub coherence: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ActivityRecord {
    pub id: i64,
    pub user_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub steps: Option<f32>,
    pub calories: Option<f32>,
    pub distance: Option<f32>,
    pub active_minutes: Option<f32>,
    pub mets: Option<f32>,
    pub activity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TimeRangeQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub granularity: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiometricUpload {
    pub records: Vec<BiometricEntry>,
}

#[derive(Debug, Deserialize)]
pub struct BiometricEntry {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
}

// ─── Validated upload DTOs ───────────────────────────────────────────────────
// Each metric gets its own typed entry so range checks live on the field itself.
// Error messages match `"must be X-Y"` for the 422 fields map.

#[derive(Debug, Deserialize, Validate)]
pub struct HeartRateEntry {
    pub timestamp: DateTime<Utc>,
    #[validate(range(min = 30.0, max = 220.0, message = "must be 30-220"))]
    pub value: f64,
}

#[derive(Debug, Deserialize, Validate)]
pub struct HeartRateUpload {
    #[validate(nested)]
    pub records: Vec<HeartRateEntry>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct SpO2Entry {
    pub timestamp: DateTime<Utc>,
    #[validate(range(min = 70.0, max = 100.0, message = "must be 70.0-100.0"))]
    pub value: f64,
}

#[derive(Debug, Deserialize, Validate)]
pub struct SpO2Upload {
    #[validate(nested)]
    pub records: Vec<SpO2Entry>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct TemperatureEntry {
    pub timestamp: DateTime<Utc>,
    #[validate(range(min = 32.0, max = 42.0, message = "must be 32.0-42.0"))]
    pub value: f64,
}

#[derive(Debug, Deserialize, Validate)]
pub struct TemperatureUpload {
    #[validate(nested)]
    pub records: Vec<TemperatureEntry>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct HRVEntry {
    pub timestamp: DateTime<Utc>,
    // Accept both `rmssd` (canonical) and `value` (simple upload payload).
    #[serde(default, alias = "value")]
    #[validate(range(min = 5.0, max = 200.0, message = "must be 5.0-200.0"))]
    pub rmssd: Option<f64>,
    pub stress: Option<f64>,
    pub heart_rate: Option<f64>,
    pub systolic_bp: Option<f64>,
    pub diastolic_bp: Option<f64>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct HRVUpload {
    #[validate(nested)]
    pub records: Vec<HRVEntry>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ActivityUpload {
    pub timestamp: Option<DateTime<Utc>>,
    #[validate(range(min = 0.0, max = 100000.0, message = "must be 0-100000"))]
    pub steps: Option<f64>,
    #[validate(range(min = 0.0, max = 50000.0, message = "must be 0.0-50000.0"))]
    pub calories: Option<f64>,
    pub distance: Option<f64>,
    pub active_minutes: Option<f64>,
    pub mets: Option<f64>,
    pub activity_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    pub device_id: Option<String>,
    pub records: Vec<SyncRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRecord {
    #[serde(rename = "type")]
    pub record_type: String,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
}
