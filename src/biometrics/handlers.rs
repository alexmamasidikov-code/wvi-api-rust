use axum::{extract::{Query, State}, Extension, Json};
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, QueryBuilder, Postgres};
use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use crate::emotions::engine::EmotionEngine;
use crate::events::{EventBus, BiometricEvent, TOPIC_BIOMETRICS};
use crate::validation::ValidatedJson;
use super::models::*;

fn default_from() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::days(7)
}

fn default_to() -> chrono::DateTime<Utc> {
    // Add 1 day buffer for timezone differences
    Utc::now() + Duration::days(1)
}

pub async fn get_user_uuid(pool: &PgPool, privy_did: &str) -> AppResult<uuid::Uuid> {
    // Fast path — user already exists.
    if let Some(uid) = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM users WHERE privy_did = $1"
    ).bind(privy_did).fetch_optional(pool).await? {
        return Ok(uid);
    }
    // Just-in-time provisioning. Before this, every biometrics/sync
    // request 404'd for users who authenticated via Privy but whose
    // iOS /auth/verify sync had earlier failed (e.g. while we were
    // still on the dead /token/verify endpoint). The JWT is already
    // verified by middleware — trust the privy_did and create the
    // row on demand. Email/name backfill happens later when
    // /auth/verify is hit, or directly via Privy's /users/{did}.
    let uid = uuid::Uuid::new_v4();
    // users.email and users.name are NOT NULL in the schema, so drop
    // in synthetic placeholders. /auth/verify will backfill with the
    // real values the next time the iOS app finishes a Privy session.
    let placeholder_email = format!("{privy_did}@provisioned.wellex");
    let placeholder_name = "Wellex User";
    sqlx::query(
        r#"INSERT INTO users (id, privy_did, email, name, created_at, updated_at)
           VALUES ($1, $2, $3, $4, NOW(), NOW())
           ON CONFLICT (privy_did) DO NOTHING"#
    )
    .bind(uid)
    .bind(privy_did)
    .bind(&placeholder_email)
    .bind(placeholder_name)
    .execute(pool)
    .await?;
    // If another request raced us to INSERT, SELECT wins.
    sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT id FROM users WHERE privy_did = $1"
    ).bind(privy_did).fetch_one(pool).await
        .map_err(|e| crate::error::AppError::Internal(
            format!("user provisioning failed: {e}")
        ))
}

// ═══ HEART RATE ═══
pub async fn get_heart_rate(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, f32, Option<f32>, Option<i32>)>(
        "SELECT timestamp, bpm, confidence, zone FROM heart_rate WHERE user_id = $1 AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "bpm": r.1, "confidence": r.2, "zone": r.3,
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

#[tracing::instrument(name = "biometrics.post_heart_rate", skip_all, fields(user = %user.privy_did, records = body.records.len()))]
pub async fn post_heart_rate(user: AuthUser, State(pool): State<PgPool>, ValidatedJson(body): ValidatedJson<HeartRateUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO heart_rate (user_id, timestamp, bpm) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32)
            .execute(&pool).await?;
        crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, r.timestamp, "hr".to_string(), r.value as f64);
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "heart_rate" } })))
}

// ═══ HRV ═══
pub async fn get_hrv(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT timestamp, rmssd, stress, heart_rate FROM hrv WHERE user_id = $1 AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "rmssd": r.1, "stress": r.2, "heartRate": r.3,
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn post_hrv(user: AuthUser, State(pool): State<PgPool>, ValidatedJson(body): ValidatedJson<HRVUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    let mut rejected_placeholder = 0i64;
    for r in &body.records {
        // Reject known JCV8 firmware placeholders: 70.0 = "no data",
        // 39.0 / 26.0 = quantized rest/stress categories (not real HRV).
        if let Some(v) = r.rmssd {
            if v == 70.0 || v == 39.0 || v == 26.0 {
                rejected_placeholder += 1;
                continue;
            }
        }
        sqlx::query("INSERT INTO hrv (user_id, timestamp, rmssd, stress, heart_rate, systolic_bp, diastolic_bp) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(uid)
            .bind(r.timestamp)
            .bind(r.rmssd.map(|v| v as f32))
            .bind(r.stress.map(|v| v as f32))
            .bind(r.heart_rate.map(|v| v as f32))
            .bind(r.systolic_bp.map(|v| v as f32))
            .bind(r.diastolic_bp.map(|v| v as f32))
            .execute(&pool).await?;
        if let Some(v) = r.rmssd {
            crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, r.timestamp, "hrv".to_string(), v);
        }
        if let Some(v) = r.stress {
            crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, r.timestamp, "stress".to_string(), v);
        }
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "rejectedPlaceholder": rejected_placeholder, "type": "hrv" } })))
}

// ═══ SpO2 ═══
pub async fn get_spo2(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, f32, Option<f32>)>(
        "SELECT timestamp, value, confidence FROM spo2 WHERE user_id = $1 AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "value": r.1, "spo2": r.1, "confidence": r.2,
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn post_spo2(user: AuthUser, State(pool): State<PgPool>, ValidatedJson(body): ValidatedJson<SpO2Upload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO spo2 (user_id, timestamp, value) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32).execute(&pool).await?;
        crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, r.timestamp, "spo2".to_string(), r.value as f64);
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "spo2" } })))
}

// ═══ TEMPERATURE ═══
pub async fn get_temperature(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, f32, Option<String>)>(
        "SELECT timestamp, value, location FROM temperature WHERE user_id = $1 AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "value": r.1, "celsius": r.1, "location": r.2,
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn post_temperature(user: AuthUser, State(pool): State<PgPool>, ValidatedJson(body): ValidatedJson<TemperatureUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO temperature (user_id, timestamp, value) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32).execute(&pool).await?;
        crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, r.timestamp, "temp".to_string(), r.value as f64);
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "temperature" } })))
}

// ═══ SLEEP ═══
pub async fn get_sleep(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.map(|d| d.date_naive()).unwrap_or_else(|| (Utc::now() - Duration::days(30)).date_naive());
    let to = q.to.map(|d| d.date_naive()).unwrap_or_else(|| Utc::now().date_naive());
    let rows = sqlx::query_as::<_, SleepRecord>(
        "SELECT id, NULL::text as user_id, date, bedtime, wake_time, total_hours, sleep_score, efficiency, deep_percent, light_percent, rem_percent, awake_percent, avg_hr, avg_hrv, avg_spo2, respiratory_rate, disturbances FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND date BETWEEN $2 AND $3 ORDER BY date DESC"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_sleep(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    sqlx::query("INSERT INTO sleep_records (user_id, date, total_hours, sleep_score, deep_percent, light_percent, rem_percent, avg_hr, avg_hrv) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)")
        .bind(uid)
        .bind(body.get("date").and_then(|v| v.as_str()).and_then(|s| s.parse::<chrono::NaiveDate>().ok()).unwrap_or_else(|| Utc::now().date_naive()))
        .bind(body.get("totalHours").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("sleepScore").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("deepPercent").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("lightPercent").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("remPercent").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("avgHR").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("avgHRV").and_then(|v| v.as_f64()).map(|v| v as f32))
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 1, "type": "sleep" } })))
}

// ═══ PPI ═══
pub async fn get_ppi(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, PPIRecord>(
        "SELECT id, NULL::text as user_id, timestamp, intervals, rmssd, coherence FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_ppi(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let rmssd = body.get("rmssd").and_then(|v| v.as_f64());
    let coherence = body.get("coherence").and_then(|v| v.as_f64());
    sqlx::query("INSERT INTO ppi (user_id, timestamp, intervals, rmssd, coherence) VALUES ($1, NOW(), $2, $3, $4)")
        .bind(uid)
        .bind(body.get("intervals"))
        .bind(rmssd.map(|v| v as f32))
        .bind(coherence.map(|v| v as f32))
        .execute(&pool).await?;
    let ts = Utc::now();
    if let Some(v) = coherence {
        crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, ts, "coherence".to_string(), v);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 1, "type": "ppi" } })))
}

// ═══ ECG ═══
//
// Project F — ECG Rework. Two sources share this table: the JCV8 bracelet's
// single-lead stream and Apple Watch imports via HKElectrocardiogram. No
// UNIQUE(user_id, timestamp) constraint exists (Project D finding: client
// timestamps are fuzzy; adding one mid-stream is risky) — dedup is achieved
// on the read path with ORDER BY timestamp DESC + client-side id hashing.

#[derive(Debug, Deserialize, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum ECGSource { Bracelet, AppleWatch }

impl ECGSource {
    pub fn as_str(&self) -> &'static str {
        match self { Self::Bracelet => "bracelet", Self::AppleWatch => "apple_watch" }
    }
}

fn default_ecg_source() -> ECGSource { ECGSource::Bracelet }

#[derive(Debug, Deserialize)]
pub struct PostECGBody {
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(alias = "durationSeconds")]
    pub duration_seconds: Option<i32>,
    #[serde(alias = "sampleRate")]
    pub sample_rate: Option<i32>,
    pub samples: Option<serde_json::Value>,
    pub analysis: Option<serde_json::Value>,
    #[serde(default = "default_ecg_source")]
    pub source: ECGSource,
}

#[derive(Debug, Deserialize)]
pub struct GetECGQuery {
    pub period: Option<String>,
    pub source: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

fn ecg_period_start(period: Option<&str>) -> DateTime<Utc> {
    match period {
        Some("30d") => Utc::now() - Duration::days(30),
        Some("3m") | Some("90d") => Utc::now() - Duration::days(90),
        Some("1y") | Some("365d") => Utc::now() - Duration::days(365),
        _ => Utc::now() - Duration::days(7),
    }
}

pub async fn get_ecg(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<GetECGQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let from = q.from.unwrap_or_else(|| ecg_period_start(q.period.as_deref()));
    let to = q.to.unwrap_or_else(default_to);
    let source_filter = q.source.as_deref().unwrap_or("all").to_string();

    let rows = sqlx::query_as::<_, (
        uuid::Uuid,
        chrono::DateTime<Utc>,
        Option<i32>,
        Option<i32>,
        String,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    )>(
        "SELECT id, timestamp, duration_seconds, sample_rate, source, analysis_json, analysis
         FROM ecg
         WHERE user_id = $1
           AND timestamp BETWEEN $2 AND $3
           AND ($4 = 'all' OR source = $4)
         ORDER BY timestamp DESC LIMIT 50"
    )
    .bind(uid).bind(from).bind(to).bind(&source_filter)
    .fetch_all(&pool).await?;

    let items: Vec<serde_json::Value> = rows.into_iter().map(|(id, ts, dur, sr, src, analysis_json, legacy)| {
        // Prefer the structured analysis_json (Project F) and fall back to the
        // legacy free-form `analysis` column so historical rows still render.
        let analysis = analysis_json.or(legacy);
        serde_json::json!({
            "id": id,
            "timestamp": ts,
            "duration_seconds": dur,
            "sample_rate": sr,
            "source": src,
            "analysis": analysis,
        })
    }).collect();

    Ok(Json(serde_json::json!({ "success": true, "data": items })))
}

pub async fn post_ecg(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<PostECGBody>,
) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let id = uuid::Uuid::new_v4();
    let ts = body.timestamp.unwrap_or_else(Utc::now);
    sqlx::query(
        "INSERT INTO ecg (id, user_id, timestamp, duration_seconds, sample_rate, samples, analysis, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(id)
    .bind(uid)
    .bind(ts)
    .bind(body.duration_seconds)
    .bind(body.sample_rate)
    .bind(body.samples)
    .bind(body.analysis)
    .bind(body.source.as_str())
    .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "id": id, "type": "ecg" } })))
}

pub async fn get_ecg_by_id(
    user: AuthUser,
    State(pool): State<PgPool>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let row = sqlx::query_as::<_, (
        chrono::DateTime<Utc>,
        Option<i32>,
        Option<i32>,
        Option<serde_json::Value>,
        String,
        Option<serde_json::Value>,
        Option<serde_json::Value>,
    )>(
        "SELECT timestamp, duration_seconds, sample_rate, samples, source, analysis_json, analysis
         FROM ecg WHERE id = $1 AND user_id = $2"
    )
    .bind(id).bind(uid)
    .fetch_optional(&pool).await?;

    match row {
        Some((ts, dur, sr, samples, src, analysis_json, legacy)) => {
            let analysis = analysis_json.or(legacy);
            Ok(Json(serde_json::json!({
                "success": true,
                "data": {
                    "id": id,
                    "timestamp": ts,
                    "duration_seconds": dur,
                    "sample_rate": sr,
                    "samples": samples,
                    "source": src,
                    "analysis": analysis,
                }
            })))
        }
        None => Err(AppError::NotFound(format!("ECG {} not found", id))),
    }
}

// ═══ ACTIVITY ═══
pub async fn get_activity(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, ActivityRecord>(
        "SELECT id, NULL::text as user_id, timestamp, steps, calories, distance, active_minutes, mets, activity_type FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_activity(user: AuthUser, State(pool): State<PgPool>, ValidatedJson(body): ValidatedJson<ActivityUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    sqlx::query("INSERT INTO activity (user_id, timestamp, steps, calories, distance, active_minutes, mets, activity_type) VALUES ($1, NOW(), $2, $3, $4, $5, $6, $7)")
        .bind(uid)
        .bind(body.steps.map(|v| v as f32))
        .bind(body.calories.map(|v| v as f32))
        .bind(body.distance.map(|v| v as f32))
        .bind(body.active_minutes.map(|v| v as f32))
        .bind(body.mets.map(|v| v as f32))
        .bind(body.activity_type.as_deref())
        .execute(&pool).await?;
    // Intraday: emit 1-min activity_intensity sample (MET proxy) + workout event if type known.
    let ts = Utc::now();
    if let Some(m) = body.mets {
        crate::intraday::ingest::spawn_write_1min(pool.clone(), uid, ts, "activity_intensity".to_string(), m);
    }
    if let Some(kind) = body.activity_type.as_deref() {
        let meta = serde_json::json!({
            "kind": kind,
            "active_minutes": body.active_minutes,
            "calories": body.calories,
            "mets": body.mets,
        });
        crate::intraday::ingest::spawn_write_event(pool.clone(), uid, ts, "workout".to_string(), meta);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 1, "type": "activity" } })))
}

// ═══ DERIVED METRICS ═══

/// Source of a BP reading. `manual` = user-typed in the sheet, `healthkit` =
/// imported from Apple Health (cuff / watch / 3rd-party device), `estimated` =
/// server-side derivation from HR+HRV used as read-through fallback only.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BPSource { Manual, HealthKit, Estimated }

impl BPSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::HealthKit => "healthkit",
            Self::Estimated => "estimated",
        }
    }
}

fn default_bp_source() -> BPSource { BPSource::Manual }

#[derive(Debug, Deserialize)]
pub struct PostBPRecord {
    pub timestamp: Option<DateTime<Utc>>,
    pub systolic: i32,
    pub diastolic: i32,
    #[serde(default = "default_bp_source")]
    pub source: BPSource,
}

#[derive(Debug, Deserialize)]
pub struct PostBPBody { pub records: Vec<PostBPRecord> }

#[derive(Debug, Deserialize)]
pub struct GetBPQuery {
    pub period: Option<String>, // "7d"|"30d"|"3m"|"1y"
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct BPReading {
    pub timestamp: DateTime<Utc>,
    pub systolic: i32,
    pub diastolic: i32,
    pub source: String,
    pub age_sec: Option<i64>,
    pub tier: String,
}

#[derive(Debug, Serialize)]
pub struct BPResult {
    pub current: Option<BPReading>,
    pub history: Vec<BPReading>,
    pub estimated_fallback: bool,
}

/// AHA 2017 clinical BP categories.
pub fn classify_tier(s: i32, d: i32) -> &'static str {
    if s >= 180 || d >= 120 { "crisis" }
    else if s >= 140 || d >= 90 { "stage2" }
    else if s >= 130 || d >= 80 { "stage1" }
    else if s >= 120 && d < 80  { "elevated" }
    else { "normal" }
}

/// Read-through estimate used only when no fresh manual/healthkit reading is
/// available. Never written to the DB — see `post_blood_pressure`. Formula is
/// the existing v1: 112/70 baseline + age/HR/HRV corrections. Bounded so a
/// degenerate HR/HRV combo can't produce impossible values.
pub fn estimate_bp(hr: f64, hrv: f64, age: i32) -> (i32, i32) {
    let baseline_s = 112.0;
    let baseline_d = 70.0;
    let age_correction = (age as f64 - 30.0) * 0.3;
    let hr_dev = (hr - 65.0) * 0.2;
    let hrv_correction = (50.0 - hrv) * 0.05;
    let s = (baseline_s + age_correction + hr_dev + hrv_correction).round() as i32;
    let d = (baseline_d + age_correction * 0.5 + hr_dev * 0.6).round() as i32;
    (s.clamp(80, 200), d.clamp(50, 130))
}

pub async fn get_blood_pressure(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<GetBPQuery>,
) -> AppResult<Json<BPResult>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let period_start = match q.period.as_deref() {
        Some("30d") => Utc::now() - Duration::days(30),
        Some("3m")  => Utc::now() - Duration::days(90),
        Some("1y")  => Utc::now() - Duration::days(365),
        _           => q.from.unwrap_or_else(|| Utc::now() - Duration::days(7)),
    };

    // Priority lookup for "current": manual > healthkit. Estimated rows live
    // outside the partial index — this query never touches them.
    let current_row: Option<(DateTime<Utc>, f32, f32, String)> = sqlx::query_as(
        "SELECT timestamp, systolic_bp::real, diastolic_bp::real, bp_source
         FROM hrv
         WHERE user_id=$1 AND bp_source IN ('manual','healthkit')
           AND systolic_bp IS NOT NULL AND diastolic_bp IS NOT NULL
         ORDER BY
           CASE bp_source WHEN 'manual' THEN 0 WHEN 'healthkit' THEN 1 END,
           timestamp DESC
         LIMIT 1"
    ).bind(user_id).fetch_optional(&pool).await?;

    let now = Utc::now();
    let (current_reading, estimated_fallback) = match current_row {
        Some((ts, s, d, src)) if now - ts < Duration::hours(6) => {
            let s_i = s.round() as i32;
            let d_i = d.round() as i32;
            (Some(BPReading {
                timestamp: ts,
                systolic: s_i,
                diastolic: d_i,
                source: src,
                age_sec: Some((now - ts).num_seconds()),
                tier: classify_tier(s_i, d_i).to_string(),
            }), false)
        }
        _ => {
            // Fallback: derive from latest HR/HRV + user age. Absence of either
            // → current stays None and the UI shows the "no data" empty state.
            let latest: Option<(DateTime<Utc>, Option<f32>, Option<f32>, Option<i32>)> = sqlx::query_as(
                "SELECT h.timestamp, h.heart_rate, h.rmssd, u.age
                 FROM hrv h JOIN users u ON u.id = h.user_id
                 WHERE h.user_id=$1 AND h.heart_rate IS NOT NULL
                 ORDER BY h.timestamp DESC LIMIT 1"
            ).bind(user_id).fetch_optional(&pool).await?;
            let reading = latest.and_then(|(ts, hr_opt, hrv_opt, age_opt)| {
                hr_opt.map(|hr| {
                    let (s, d) = estimate_bp(
                        hr as f64,
                        hrv_opt.unwrap_or(50.0) as f64,
                        age_opt.unwrap_or(30),
                    );
                    BPReading {
                        timestamp: ts,
                        systolic: s,
                        diastolic: d,
                        source: "estimated".to_string(),
                        age_sec: Some((now - ts).num_seconds()),
                        tier: classify_tier(s, d).to_string(),
                    }
                })
            });
            (reading, true)
        }
    };

    // History is manual+healthkit only — estimated is a read-through synth,
    // not a record, so it never appears here.
    let history_rows: Vec<(DateTime<Utc>, f32, f32, String)> = sqlx::query_as(
        "SELECT timestamp, systolic_bp::real, diastolic_bp::real, bp_source FROM hrv
         WHERE user_id=$1 AND bp_source IN ('manual','healthkit')
           AND timestamp >= $2
           AND systolic_bp IS NOT NULL AND diastolic_bp IS NOT NULL
         ORDER BY timestamp DESC
         LIMIT 500"
    ).bind(user_id).bind(period_start).fetch_all(&pool).await?;

    let history: Vec<BPReading> = history_rows.into_iter().map(|(ts, s, d, src)| {
        let s_i = s.round() as i32;
        let d_i = d.round() as i32;
        BPReading {
            timestamp: ts,
            systolic: s_i,
            diastolic: d_i,
            source: src,
            age_sec: Some((now - ts).num_seconds()),
            tier: classify_tier(s_i, d_i).to_string(),
        }
    }).collect();

    Ok(Json(BPResult { current: current_reading, history, estimated_fallback }))
}

/// POST /api/v1/biometrics/blood-pressure — multi-source BP ingest.
///
/// Accepts `{ records: [{ timestamp, systolic, diastolic, source }] }` with
/// `source ∈ {manual, healthkit}` (defaults to `manual`). `estimated` payloads
/// from the client are ignored silently — estimated is a GET-only synth.
/// Writes land in the existing `hrv` table (systolic_bp/diastolic_bp columns)
/// alongside the new `bp_source` column introduced by migration 010.
pub async fn post_blood_pressure(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<PostBPBody>,
) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::users::resolve_user_id(&pool, &user.privy_did).await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut inserted = 0i64;
    let mut rejected = 0i64;
    let mut crisis_detected: Option<(i32, i32, DateTime<Utc>)> = None;

    for rec in body.records {
        // Physiological bounds + sys>dia sanity. Matches BPManualEntrySheet's
        // client-side ranges so we don't silently diverge.
        if !(70..=250).contains(&rec.systolic)
            || !(40..=150).contains(&rec.diastolic)
            || rec.systolic <= rec.diastolic
        {
            rejected += 1;
            continue;
        }
        // Estimated readings are server-side derived at read time only. A
        // client shouldn't try to post them; if it does, we drop them.
        if matches!(rec.source, BPSource::Estimated) {
            rejected += 1;
            continue;
        }

        let ts = rec.timestamp.unwrap_or_else(Utc::now);
        let src = rec.source.as_str();

        // hrv has no unique (user_id, timestamp) constraint — can't ON CONFLICT.
        // Source priority (manual > healthkit > estimated) is enforced at GET
        // time via ORDER BY on bp_source, not at write time.
        sqlx::query(
            "INSERT INTO hrv (user_id, timestamp, systolic_bp, diastolic_bp, bp_source)
             VALUES ($1, $2, $3, $4, $5)"
        )
        .bind(uid)
        .bind(ts)
        .bind(rec.systolic as f32)
        .bind(rec.diastolic as f32)
        .bind(src)
        .execute(&pool).await?;
        inserted += 1;

        // Track worst crisis observed in this batch so we dispatch at most one
        // push per request.
        if rec.systolic >= 180 || rec.diastolic >= 120 {
            crisis_detected = Some((rec.systolic, rec.diastolic, ts));
        }

        // Intraday hook (Project A): write one 1-min sample per BP component
        // so the detail screen chart picks them up on the next fetch.
        crate::intraday::ingest::spawn_write_1min(
            pool.clone(), uid, ts, "systolic_bp".to_string(), rec.systolic as f64
        );
        crate::intraday::ingest::spawn_write_1min(
            pool.clone(), uid, ts, "diastolic_bp".to_string(), rec.diastolic as f64
        );
    }

    // Crisis push is fire-and-forget — the response to the client must not
    // wait on APNs round-trip latency.
    if let Some((s, d, _)) = crisis_detected {
        let pool_c = pool.clone();
        tokio::spawn(async move {
            if let Err(e) = dispatch_bp_crisis(&pool_c, uid, s, d).await {
                tracing::error!(?e, "bp_crisis dispatch failed");
            }
        });
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "recordsSaved": inserted,
            "rejected": rejected,
            "type": "blood_pressure",
        }
    })))
}

/// Dispatch a BP crisis push if we haven't sent one in the last 4h. Uses
/// `push_notifications_log` as a lightweight dedup store and the existing
/// `ApnsClient::send_alert` path (no new APNs abstraction invented).
async fn dispatch_bp_crisis(
    pool: &PgPool,
    user_id: uuid::Uuid,
    systolic: i32,
    diastolic: i32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 4h dedup window — stops us paging a user every minute while their
    // bracelet streams identical readings.
    let last: Option<(DateTime<Utc>,)> = sqlx::query_as(
        "SELECT sent_at FROM push_notifications_log
         WHERE user_id=$1 AND category='bp_crisis'
         ORDER BY sent_at DESC LIMIT 1"
    ).bind(user_id).fetch_optional(pool).await?;
    if let Some((last_ts,)) = last {
        if Utc::now() - last_ts < Duration::hours(4) {
            tracing::info!(?user_id, "bp_crisis dedup: last push <4h ago");
            return Ok(());
        }
    }

    let title = format!("⚠️ Критическое давление {}/{}", systolic, diastolic);
    let body = "Серьёзные показатели давления — обратись к врачу".to_string();

    // Fetch all active push tokens for this user.
    let tokens: Vec<String> = sqlx::query_scalar(
        "SELECT token FROM push_tokens WHERE user_id=$1"
    ).bind(user_id).fetch_all(pool).await.unwrap_or_default();

    if tokens.is_empty() {
        tracing::info!(?user_id, "bp_crisis: no push tokens — skip");
        return Ok(());
    }

    // One-shot APNs client for this dispatch. Reusing the scheduler's client
    // would require piping it in as state; for a fire-and-forget path we
    // construct locally — the ApnsConfig::from_env() inside is cheap and the
    // JWT cache is per-client but only re-built every 50min anyway.
    let apns = crate::push::apns::ApnsClient::new();
    let deeplink = "wellex://body/bp";
    let mut sent = 0;
    for token in &tokens {
        match apns.send_alert(token, &title, &body, Some(deeplink)).await {
            Ok(()) => sent += 1,
            Err(e) => tracing::warn!(?user_id, "apns send failed: {e}"),
        }
    }

    if sent > 0 {
        sqlx::query(
            "INSERT INTO push_notifications_log (user_id, category) VALUES ($1, 'bp_crisis')"
        ).bind(user_id).execute(pool).await?;
    }

    Ok(())
}

pub async fn get_stress(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>)>(
        "SELECT timestamp, stress FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND stress IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "stress": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_breathing_rate(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (Option<f32>,)>(
        "SELECT respiratory_rate FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "breathingRate": row.and_then(|r| r.0) } })))
}

pub async fn get_rmssd(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>)>(
        "SELECT timestamp, rmssd FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND rmssd IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "rmssd": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_coherence(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>)>(
        "SELECT timestamp, coherence FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND coherence IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "coherence": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

// ═══ RECOVERY ═══
pub async fn get_recovery(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;

    // Get latest HRV (morning reading)
    let morning_hrv = sqlx::query_scalar::<_, f32>(
        "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(0.0) as f64;

    // Get 7-day HRV baseline (average)
    let baseline_hrv = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(AVG(rmssd)::float8, 0) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0.0);

    // Get last night sleep score
    let sleep_score = sqlx::query_scalar::<_, f32>(
        "SELECT COALESCE(sleep_score, 0) FROM sleep_records WHERE user_id = $1 ORDER BY date DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(0.0) as f64;

    // Count days of HRV data
    let hrv_days = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT DATE(timestamp)) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0);

    // Need at least morning HRV to calculate
    if morning_hrv == 0.0 {
        return Ok(Json(serde_json::json!({
            "success": true,
            "data": {
                "ready": false,
                "reason": "No HRV data available. Wear bracelet and stay still for 60 seconds.",
                "recoveryPercent": null,
            }
        })));
    }

    // Calculate recovery
    let hrv_ratio = if baseline_hrv > 0.0 { morning_hrv / baseline_hrv } else { 1.0 };

    // Recovery formula:
    // - HRV vs baseline: 40% weight
    // - Sleep quality: 40% weight (equal to HRV — sleep is critical for recovery)
    // - Absolute HRV quality: 10% weight
    // - Resting HR bonus: 10% weight
    let hrv_component = (hrv_ratio * 100.0).clamp(0.0, 120.0) * 0.4;
    let sleep_component = sleep_score * 0.4;
    let abs_component = (morning_hrv / 100.0 * 100.0).clamp(0.0, 100.0) * 0.1;

    // Resting HR component: lower = better recovery
    let resting_hr = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(70.0) as f64;
    let hr_component = if resting_hr < 60.0 { 100.0 }
        else if resting_hr < 70.0 { 80.0 }
        else if resting_hr < 80.0 { 60.0 }
        else { 40.0 };
    let hr_weighted = hr_component * 0.1;

    let mut recovery = (hrv_component + sleep_component + abs_component + hr_weighted).clamp(0.0, 100.0);

    // SLEEP CAP: poor sleep = limited recovery regardless of HRV
    if sleep_score > 0.0 && sleep_score < 50.0 {
        recovery = recovery.min(65.0); // Can't be fully recovered with poor sleep
    }
    if sleep_score > 0.0 && sleep_score < 30.0 {
        recovery = recovery.min(45.0); // Very poor sleep = max 45% recovery
    }

    let recovery = recovery.round();

    let level = if recovery >= 80.0 { "excellent" }
        else if recovery >= 60.0 { "good" }
        else if recovery >= 40.0 { "moderate" }
        else { "poor" };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "ready": true,
            "recoveryPercent": recovery,
            "level": level,
            "morningHRV": morning_hrv,
            "baselineHRV": if baseline_hrv > 0.0 { Some(baseline_hrv) } else { None::<f64> },
            "hrvRatio": hrv_ratio,
            "sleepScore": sleep_score,
            "daysOfData": hrv_days,
            "note": if hrv_days < 3 { "Recovery accuracy improves with more days of data" } else { "Based on your personal baseline" },
        }
    })))
}

// ═══ COMPUTED METRICS ═══
pub async fn get_computed(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    use super::computed;
    let uid = get_user_uuid(&pool, &user.privy_did).await?;

    let hr_opt = sqlx::query_scalar::<_, f32>("SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?;
    let hr = hr_opt.unwrap_or(0.0) as f64;

    let hrv_opt = sqlx::query_scalar::<_, f32>("SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?;
    let hrv = hrv_opt.unwrap_or(0.0) as f64;

    let spo2_opt = sqlx::query_scalar::<_, f32>("SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?;
    let spo2 = spo2_opt.unwrap_or(0.0) as f64;

    let steps = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(steps)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'")
        .bind(uid).fetch_one(&pool).await? as f64;
    let active_min_raw = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(active_minutes)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0);
    let active_min = (active_min_raw as f64).min(120.0);

    // Fetch latest sleep data for sleep score
    let sleep_row = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT deep_percent, rem_percent, total_hours, awake_percent FROM sleep_records WHERE user_id = $1 ORDER BY date DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?;
    let sleep_score = if let Some((deep, rem, hours, awake)) = sleep_row {
        computed::compute_sleep_score(
            deep.unwrap_or(20.0) as f64,
            rem.unwrap_or(20.0) as f64,
            hours.unwrap_or(7.5) as f64,
            awake.unwrap_or(5.0) as f64,
        )
    } else {
        0.0
    };

    let age = 30.0; // Default — should come from user profile

    // Use fallback values for non-bio-age computed metrics so they still work
    let hr_for_compute = if hr > 0.0 { hr } else { 70.0 };
    let hrv_for_compute = if hrv > 0.0 { hrv } else { 50.0 };
    let spo2_for_compute = if spo2 > 0.0 { spo2 } else { 98.0 };
    let sleep_score_for_display = if sleep_score > 0.0 { sleep_score } else { 75.0 };

    let (sys, dia) = computed::estimate_blood_pressure(hr_for_compute, hrv_for_compute);
    let vo2_max = computed::estimate_vo2_max(hr_for_compute, age);
    let coherence = computed::compute_coherence(hrv_for_compute);
    let training_load = computed::compute_training_load(active_min, hr_for_compute, 220.0 - age);

    // Bio Age requires 7 days of accumulated data
    let days_with_hr = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT DATE(timestamp)) FROM heart_rate WHERE user_id = $1"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0);

    let days_with_hrv = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT DATE(timestamp)) FROM hrv WHERE user_id = $1"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0);

    let days_with_sleep = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sleep_records WHERE user_id = $1"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0);

    let bio_age_ready = days_with_hr >= 7 && days_with_hrv >= 7 && days_with_sleep >= 7;

    let bio_age = if bio_age_ready {
        // Use 7-day averages for stable calculation
        let avg_hr = sqlx::query_scalar::<_, f64>(
            "SELECT COALESCE(AVG(bpm)::float8, 70) FROM heart_rate WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
        ).bind(uid).fetch_one(&pool).await.unwrap_or(70.0);

        let avg_hrv = sqlx::query_scalar::<_, f64>(
            "SELECT COALESCE(AVG(rmssd)::float8, 50) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
        ).bind(uid).fetch_one(&pool).await.unwrap_or(50.0);

        let avg_spo2 = sqlx::query_scalar::<_, f64>(
            "SELECT COALESCE(AVG(value)::float8, 98) FROM spo2 WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
        ).bind(uid).fetch_one(&pool).await.unwrap_or(98.0);

        let avg_steps = sqlx::query_scalar::<_, f64>(
            "SELECT COALESCE(AVG(daily_steps)::float8, 0) FROM (SELECT DATE(timestamp), SUM(steps) as daily_steps FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days' GROUP BY DATE(timestamp)) sub"
        ).bind(uid).fetch_one(&pool).await.unwrap_or(0.0);

        let avg_sleep = sqlx::query_scalar::<_, f64>(
            "SELECT COALESCE(AVG(sleep_score)::float8, 0) FROM sleep_records WHERE user_id = $1 AND date >= CURRENT_DATE - 7"
        ).bind(uid).fetch_one(&pool).await.unwrap_or(0.0);

        Some(computed::compute_bio_age(age, avg_hr, avg_hrv, avg_spo2, avg_steps, avg_sleep))
    } else {
        None
    };

    let mut requirements: Vec<String> = vec![];
    if days_with_hr < 7 { requirements.push(format!("HR data: {}/7 days", days_with_hr)); }
    if days_with_hrv < 7 { requirements.push(format!("HRV data: {}/7 days", days_with_hrv)); }
    if days_with_sleep < 7 { requirements.push(format!("Sleep data: {}/7 nights", days_with_sleep)); }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "bloodPressure": { "systolic": sys, "diastolic": dia },
            "vo2Max": (vo2_max * 10.0).round() / 10.0,
            "coherence": coherence.round(),
            "bioAge": bio_age,
            "bioAgeReady": bio_age_ready,
            "bioAgeRequirements": requirements,
            "trainingLoad": training_load,
            "sleepScore": sleep_score_for_display,
        }
    })))
}

// ═══ REALTIME SNAPSHOT ═══
pub async fn get_realtime(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let hr = sqlx::query_as::<_, (f32,)>("SELECT bpm FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;
    let hrv = sqlx::query_as::<_, (Option<f32>, Option<f32>)>("SELECT rmssd, stress FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;
    let spo2 = sqlx::query_as::<_, (f32,)>("SELECT value FROM spo2 WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;
    let temp = sqlx::query_as::<_, (f32,)>("SELECT value FROM temperature WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "heartRate": hr.map(|r| r.0),
            "hrv": hrv.as_ref().and_then(|r| r.0),
            "stress": hrv.as_ref().and_then(|r| r.1),
            "spo2": spo2.map(|r| r.0),
            "temperature": temp.map(|r| r.0),
            "timestamp": Utc::now(),
        }
    })))
}

// ═══ DAILY SUMMARY ═══
pub async fn get_summary(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let today = Utc::now().date_naive();
    let start = today.and_hms_opt(0, 0, 0).unwrap().and_utc();

    let hr = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>)>(
        "SELECT AVG(bpm)::float8, MIN(bpm)::float8, MAX(bpm)::float8 FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= $2"
    ).bind(&user.privy_did).bind(start).fetch_one(&pool).await?;

    let act = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>)>(
        "SELECT SUM(steps)::float8, SUM(calories)::float8, SUM(active_minutes)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= $2"
    ).bind(&user.privy_did).bind(start).fetch_one(&pool).await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "date": today,
            "hr": { "avg": hr.0, "min": hr.1, "max": hr.2 },
            "steps": act.0.unwrap_or(0.0),
            "calories": act.1.unwrap_or(0.0),
            "activeMinutes": act.2.unwrap_or(0.0),
        }
    })))
}

// ═══ BULK SYNC ═══
pub async fn sync(user: AuthUser, State(pool): State<PgPool>, Extension(event_bus): Extension<EventBus>, Json(body): Json<SyncRequest>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut processed = 0i64;
    let received = body.records.len();

    // Track latest values seen in this batch for emotion detection
    let mut latest_hr: Option<f64> = None;
    let mut latest_hrv: Option<f64> = None;
    let mut latest_stress: Option<f64> = None;
    let mut latest_systolic_bp: Option<f64> = None;
    let mut latest_ppi_rmssd: Option<f64> = None;
    let mut latest_ppi_coherence: Option<f64> = None;
    let mut latest_spo2: Option<f64> = None;
    let mut latest_temp: Option<f64> = None;
    let mut latest_steps: Option<f64> = None;
    let mut latest_sleep_score: Option<f64> = None;

    // Group records by type for batch INSERT — 680K single-row inserts would
    // produce 680K round-trips and time out. Multi-value INSERT with up to
    // 1000 tuples per statement cuts that to ~680 statements.
    let f = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_f64());
    let fo = |v: &serde_json::Value, k: &str| f(v, k).map(|x| x as f32);

    let mut hr_rows: Vec<(chrono::DateTime<Utc>, f32)> = Vec::new();
    let mut hrv_rows: Vec<(chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>)> = Vec::new();
    let mut spo2_rows: Vec<(chrono::DateTime<Utc>, f32)> = Vec::new();
    let mut temp_rows: Vec<(chrono::DateTime<Utc>, f32)> = Vec::new();
    let mut act_rows: Vec<(chrono::DateTime<Utc>, Option<f32>, Option<f32>, Option<f32>, Option<f32>)> = Vec::new();

    for rec in &body.records {
        match rec.record_type.as_str() {
            "heart_rate" => {
                if let Some(bpm) = f(&rec.data, "bpm") {
                    hr_rows.push((rec.timestamp, bpm as f32));
                    latest_hr = Some(bpm);
                    processed += 1;
                }
            }
            "hrv" => {
                let rmssd_opt = f(&rec.data, "rmssd");
                // Drop JCV8 firmware placeholder of exactly 70.0 ms.
                if rmssd_opt == Some(70.0) { continue; }
                hrv_rows.push((
                    rec.timestamp,
                    rmssd_opt.map(|v| v as f32),
                    fo(&rec.data, "stress"),
                    fo(&rec.data, "heartRate"),
                    fo(&rec.data, "systolicBP"),
                    fo(&rec.data, "diastolicBP"),
                ));
                if let Some(v) = rmssd_opt { latest_hrv = Some(v); }
                if let Some(v) = f(&rec.data, "stress") { latest_stress = Some(v); }
                if let Some(v) = f(&rec.data, "systolicBP") { latest_systolic_bp = Some(v); }
                if let Some(v) = f(&rec.data, "heartRate") {
                    if latest_hr.is_none() { latest_hr = Some(v); }
                }
                processed += 1;
            }
            "spo2" => {
                if let Some(val) = f(&rec.data, "value") {
                    spo2_rows.push((rec.timestamp, val as f32));
                    latest_spo2 = Some(val);
                    processed += 1;
                }
            }
            "temperature" => {
                if let Some(val) = f(&rec.data, "value") {
                    // Physiological wrist-skin range 32-42°C — drop off-wrist noise.
                    if !(32.0..=42.0).contains(&val) { continue; }
                    temp_rows.push((rec.timestamp, val as f32));
                    latest_temp = Some(val);
                    processed += 1;
                }
            }
            "activity" => {
                act_rows.push((
                    rec.timestamp,
                    fo(&rec.data, "steps"),
                    fo(&rec.data, "calories"),
                    fo(&rec.data, "activeMinutes"),
                    fo(&rec.data, "mets"),
                ));
                if let Some(v) = f(&rec.data, "steps") { latest_steps = Some(v); }
                processed += 1;
            }
            "ppi" => {
                if let Some(v) = f(&rec.data, "rmssd") { latest_ppi_rmssd = Some(v); }
                if let Some(v) = f(&rec.data, "coherence") { latest_ppi_coherence = Some(v); }
            }
            "sleep" => {
                if let Some(v) = f(&rec.data, "sleepScore") { latest_sleep_score = Some(v); }
            }
            _ => {}
        }
    }

    // Chunked multi-value INSERTs. Postgres bind-param limit is 65535; keep
    // well under it with 1000 rows per statement (max 7 params × 1000 = 7000).
    const CHUNK: usize = 1000;

    for chunk in hr_rows.chunks(CHUNK) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("INSERT INTO heart_rate (user_id, timestamp, bpm) ");
        qb.push_values(chunk, |mut b, r| { b.push_bind(uid).push_bind(r.0).push_bind(r.1); });
        qb.build().execute(&pool).await?;
    }
    for chunk in hrv_rows.chunks(CHUNK) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("INSERT INTO hrv (user_id, timestamp, rmssd, stress, heart_rate, systolic_bp, diastolic_bp) ");
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(uid).push_bind(r.0).push_bind(r.1).push_bind(r.2).push_bind(r.3).push_bind(r.4).push_bind(r.5);
        });
        qb.build().execute(&pool).await?;
    }
    for chunk in spo2_rows.chunks(CHUNK) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("INSERT INTO spo2 (user_id, timestamp, value) ");
        qb.push_values(chunk, |mut b, r| { b.push_bind(uid).push_bind(r.0).push_bind(r.1); });
        qb.build().execute(&pool).await?;
    }
    for chunk in temp_rows.chunks(CHUNK) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("INSERT INTO temperature (user_id, timestamp, value) ");
        qb.push_values(chunk, |mut b, r| { b.push_bind(uid).push_bind(r.0).push_bind(r.1); });
        qb.build().execute(&pool).await?;
    }
    for chunk in act_rows.chunks(CHUNK) {
        let mut qb: QueryBuilder<Postgres> = QueryBuilder::new("INSERT INTO activity (user_id, timestamp, steps, calories, active_minutes, mets) ");
        qb.push_values(chunk, |mut b, r| {
            b.push_bind(uid).push_bind(r.0).push_bind(r.1).push_bind(r.2).push_bind(r.3).push_bind(r.4);
        });
        qb.build().execute(&pool).await?;
    }

    // ── Emotion detection ──────────────────────────────────────────────────────
    // Run after all records are stored. Errors are non-fatal: sync always succeeds.
    if processed > 0 {
        let emotion_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
            // Fetch personal norms (baselines) — fall back to population defaults
            let norms = sqlx::query_as::<_, (Option<f32>, Option<f32>)>(
                "SELECT resting_hr, base_temp FROM personal_norms WHERE user_id = $1"
            )
            .bind(uid)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();
            let resting_hr = norms.as_ref().and_then(|n| n.0).unwrap_or(65.0) as f64;
            let base_temp  = norms.as_ref().and_then(|n| n.1).unwrap_or(36.6) as f64;

            // Fetch previous emotion for temporal smoothing
            let prev_emotion_str = sqlx::query_scalar::<_, String>(
                "SELECT primary_emotion FROM emotions WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
            )
            .bind(uid)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            // Fetch elapsed seconds since last emotion record
            let elapsed_secs = sqlx::query_scalar::<_, f64>(
                "SELECT EXTRACT(EPOCH FROM (NOW() - timestamp))::float8 FROM emotions WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
            )
            .bind(uid)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(9999.0);

            // Resolve previous EmotionState from string
            use crate::emotions::models::EmotionState;
            let prev_emotion: Option<EmotionState> = prev_emotion_str.as_deref().and_then(|s| {
                match s {
                    "calm"        => Some(EmotionState::Calm),
                    "relaxed"     => Some(EmotionState::Relaxed),
                    "joyful"      => Some(EmotionState::Joyful),
                    "energized"   => Some(EmotionState::Energized),
                    "excited"     => Some(EmotionState::Excited),
                    "focused"     => Some(EmotionState::Focused),
                    "meditative"  => Some(EmotionState::Meditative),
                    "recovering"  => Some(EmotionState::Recovering),
                    "drowsy"      => Some(EmotionState::Drowsy),
                    "stressed"    => Some(EmotionState::Stressed),
                    "anxious"     => Some(EmotionState::Anxious),
                    "angry"       => Some(EmotionState::Angry),
                    "frustrated"  => Some(EmotionState::Frustrated),
                    "fearful"     => Some(EmotionState::Fearful),
                    "sad"         => Some(EmotionState::Sad),
                    "exhausted"   => Some(EmotionState::Exhausted),
                    "pain"        => Some(EmotionState::Pain),
                    "flow"        => Some(EmotionState::Flow),
                    _             => None,
                }
            });

            // Fill in any missing values from DB if not present in the batch
            let heart_rate = match latest_hr {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(70.0) as f64,
            };
            let hrv = match latest_hrv {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(50.0) as f64,
            };
            let stress = match latest_stress {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT stress FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(30.0) as f64,
            };
            let spo2 = match latest_spo2 {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(98.0) as f64,
            };
            let temperature = match latest_temp {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT value FROM temperature WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(base_temp as f32) as f64,
            };
            let systolic_bp  = latest_systolic_bp.unwrap_or(120.0);
            let ppi_coherence = latest_ppi_coherence.unwrap_or(0.5);
            let ppi_rmssd     = latest_ppi_rmssd.unwrap_or(hrv);
            let sleep_score   = latest_sleep_score.unwrap_or(60.0);
            let activity_score = match latest_steps {
                Some(steps) => (steps / 100.0).min(100.0),
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT COALESCE(SUM(steps), 0)::float4 FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().map(|s| (s / 100.0).min(100.0)).unwrap_or(50.0) as f64,
            };

            let result = EmotionEngine::detect(
                heart_rate, resting_hr, hrv, stress,
                spo2, temperature, base_temp,
                systolic_bp, ppi_coherence, ppi_rmssd,
                sleep_score, activity_score,
                0.0, // hrv_trend: stable (not enough history in a single sync)
                prev_emotion, elapsed_secs,
            );

            let primary_str   = format!("{:?}", result.primary).to_lowercase();
            let secondary_str = format!("{:?}", result.secondary).to_lowercase();
            let all_scores_json = serde_json::to_value(&result.all_scores).unwrap_or(serde_json::Value::Null);

            sqlx::query(
                "INSERT INTO emotions (user_id, timestamp, primary_emotion, primary_confidence, secondary_emotion, secondary_confidence, all_scores) \
                 VALUES ($1, NOW(), $2, $3, $4, $5, $6)"
            )
            .bind(uid)
            .bind(&primary_str)
            .bind(result.primary_confidence as f32)
            .bind(&secondary_str)
            .bind(result.secondary_confidence as f32)
            .bind(all_scores_json)
            .execute(&pool)
            .await?;

            Ok(())
        }.await;

        let emotion_ok = emotion_result.is_ok();
        if let Err(e) = emotion_result {
            tracing::warn!("Emotion detection failed after sync (non-fatal): {}", e);
        }

        // ── Auto-trigger WVI calculation ───────────────────────────────────────
        let wvi_result: Result<(), Box<dyn std::error::Error + Send + Sync>> = async {
            use crate::wvi::calculator::{WviV2Calculator, WviV2Input};

            let hr = match latest_hr {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(72.0) as f64,
            };
            let hrv_val = match latest_hrv {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(50.0) as f64,
            };
            let stress_val = match latest_stress {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT stress FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(30.0) as f64,
            };
            let spo2_val = match latest_spo2 {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(98.0) as f64,
            };
            let temp_val = match latest_temp {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT value FROM temperature WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(36.6) as f64,
            };
            let systolic_bp_val = latest_systolic_bp.unwrap_or(120.0);
            let diastolic_bp_val = sqlx::query_scalar::<_, f32>(
                "SELECT diastolic_bp FROM hrv WHERE user_id = $1 AND diastolic_bp IS NOT NULL ORDER BY timestamp DESC LIMIT 1"
            ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(80.0) as f64;
            let ppi_coherence_val = latest_ppi_coherence.unwrap_or(0.4);
            let steps_val = match latest_steps {
                Some(v) => v,
                None => sqlx::query_scalar::<_, f32>(
                    "SELECT COALESCE(SUM(steps), 0)::float4 FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'"
                ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(0.0) as f64,
            };
            let sleep_row = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>)>(
                "SELECT total_hours, deep_percent, efficiency FROM sleep_records WHERE user_id = $1 ORDER BY date DESC LIMIT 1"
            ).bind(uid).fetch_optional(&pool).await.ok().flatten();

            // Compute sleep score from components
            let sleep_score = {
                let total_hours = sleep_row.as_ref().and_then(|r| r.0).unwrap_or(7.0) as f64;
                let deep_pct = sleep_row.as_ref().and_then(|r| r.1).unwrap_or(20.0) as f64;
                let efficiency = sleep_row.as_ref().and_then(|r| r.2).unwrap_or(85.0) as f64;
                let deep_s = if (15.0..=25.0).contains(&deep_pct) { 100.0 }
                    else { (100.0 - (deep_pct - 20.0).abs() * 5.0).max(0.0) };
                let dur_s = if (7.0..=9.0).contains(&total_hours) { 100.0 }
                    else { (100.0 - (total_hours - 8.0).abs() * 20.0).max(0.0) };
                let eff_s = (efficiency / 100.0 * 100.0).clamp(0.0, 100.0);
                deep_s * 0.35 + dur_s * 0.40 + eff_s * 0.25
            };

            // Fetch latest emotion name
            let emotion_name = sqlx::query_scalar::<_, String>(
                "SELECT primary_emotion FROM emotions WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
            ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or_default();

            // Active calories from today
            let active_calories = sqlx::query_scalar::<_, f32>(
                "SELECT COALESCE(SUM(calories), 0)::float4 FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'"
            ).bind(uid).fetch_optional(&pool).await.ok().flatten().unwrap_or(0.0) as f64;

            let input = WviV2Input {
                hrv_rmssd: hrv_val,
                stress_index: stress_val,
                sleep_score,
                emotion_score: 50.0, // default during sync
                spo2: spo2_val,
                heart_rate: hr,
                resting_hr: 65.0,
                steps: steps_val,
                active_calories,
                acwr: 1.0, // default during sync
                bp_systolic: systolic_bp_val,
                bp_diastolic: diastolic_bp_val,
                temp_delta: temp_val - 36.6,
                ppi_coherence: ppi_coherence_val,
                emotion_name,
            };

            let result = WviV2Calculator::calculate(&input);

            sqlx::query(
                "INSERT INTO wvi_scores (user_id, timestamp, wvi_score, level, metrics, weights, emotion_feedback) \
                 VALUES ($1, NOW(), $2, $3, $4, $5, $6)"
            )
            .bind(uid)
            .bind(result.wvi_score as f32)
            .bind(&result.level)
            .bind(serde_json::to_value(&result.metric_scores).unwrap_or_default())
            .bind(serde_json::json!({ "version": "2.0", "type": "geometric_weighted" }))
            .bind(result.emotion_multiplier as f32)
            .execute(&pool)
            .await?;

            Ok(())
        }.await;

        if let Err(e) = wvi_result {
            tracing::warn!("WVI auto-calculation failed after sync (non-fatal): {}", e);
        }

        tracing::info!(
            "Sync complete: {} records processed, emotion: {}, wvi: calculated",
            processed,
            if emotion_ok { "detected" } else { "failed" }
        );
    }

    crate::audit::log_action(
        &pool, &user.privy_did, "biometrics.sync", "biometrics", None,
        Some(serde_json::json!({ "recordsReceived": received, "recordsProcessed": processed })),
        None, None,
    ).await;

    // Publish biometric sync event to Kafka
    if processed > 0 {
        let event = BiometricEvent {
            user_id: uid.to_string(),
            event_type: "biometrics.sync".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            data: serde_json::json!({
                "records_received": received,
                "records_processed": processed,
                "device_id": &body.device_id,
            }),
        };
        event_bus.publish(TOPIC_BIOMETRICS, &uid.to_string(), &event).await;
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "syncId": uuid::Uuid::new_v4(), "recordsReceived": received, "recordsProcessed": processed, "deviceId": body.device_id }
    })))
}

// ═══ BP UNIT TESTS ═══
// Pure-function coverage for tier classification and the read-through estimate.
// Integration tests that hit Postgres live in `tests/` and require a live DB.
#[cfg(test)]
mod bp_tests {
    use super::*;

    #[test]
    fn bp_tier_normal() {
        assert_eq!(classify_tier(115, 75), "normal");
        assert_eq!(classify_tier(119, 79), "normal");
    }

    #[test]
    fn bp_tier_elevated() {
        assert_eq!(classify_tier(125, 75), "elevated");
        assert_eq!(classify_tier(129, 79), "elevated");
    }

    #[test]
    fn bp_tier_stage1() {
        assert_eq!(classify_tier(135, 85), "stage1");
        assert_eq!(classify_tier(130, 80), "stage1");
        // High diastolic alone trips stage1
        assert_eq!(classify_tier(118, 82), "stage1");
    }

    #[test]
    fn bp_tier_stage2() {
        assert_eq!(classify_tier(145, 95), "stage2");
        assert_eq!(classify_tier(140, 90), "stage2");
        assert_eq!(classify_tier(118, 95), "stage2");
    }

    #[test]
    fn bp_tier_crisis() {
        assert_eq!(classify_tier(185, 125), "crisis");
        assert_eq!(classify_tier(180, 90), "crisis");
        assert_eq!(classify_tier(140, 120), "crisis");
    }

    #[test]
    fn bp_estimate_baseline_30yo() {
        let (s, d) = estimate_bp(65.0, 50.0, 30);
        assert!((110..=114).contains(&s), "expected ~112, got {s}");
        assert!((68..=72).contains(&d), "expected ~70, got {d}");
    }

    #[test]
    fn bp_estimate_high_hr_low_hrv_raises_sys() {
        let (baseline_s, _) = estimate_bp(65.0, 50.0, 30);
        let (high_s, _) = estimate_bp(100.0, 20.0, 30);
        assert!(high_s > baseline_s, "high HR + low HRV should raise sys ({high_s} vs {baseline_s})");
    }

    #[test]
    fn bp_estimate_clamped_within_bounds() {
        let (s_lo, d_lo) = estimate_bp(0.0, 200.0, 18);
        let (s_hi, d_hi) = estimate_bp(250.0, 0.0, 90);
        assert!((80..=200).contains(&s_lo));
        assert!((50..=130).contains(&d_lo));
        assert!((80..=200).contains(&s_hi));
        assert!((50..=130).contains(&d_hi));
    }

    #[test]
    fn bp_source_serde_lowercase() {
        assert_eq!(BPSource::Manual.as_str(), "manual");
        assert_eq!(BPSource::HealthKit.as_str(), "healthkit");
        assert_eq!(BPSource::Estimated.as_str(), "estimated");
        let json = serde_json::to_string(&BPSource::HealthKit).unwrap();
        assert_eq!(json, "\"healthkit\"");
        let back: BPSource = serde_json::from_str("\"manual\"").unwrap();
        assert_eq!(back, BPSource::Manual);
    }
}
