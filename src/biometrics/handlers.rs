use axum::{extract::{Query, State}, Json};
use chrono::{Utc, Duration};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use super::models::*;

fn default_from() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::days(7)
}

async fn get_user_uuid(pool: &PgPool, privy_did: &str) -> AppResult<uuid::Uuid> {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1")
        .bind(privy_did)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))
}

// ═══ HEART RATE ═══
pub async fn get_heart_rate(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, HeartRateRecord>(
        "SELECT id, NULL::text as user_id, timestamp, bpm, confidence, zone FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_heart_rate(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<BiometricUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO heart_rate (user_id, timestamp, bpm) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32)
            .execute(&pool).await?;
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "heart_rate" } })))
}

// ═══ HRV ═══
pub async fn get_hrv(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, HRVRecord>(
        "SELECT id, NULL::text as user_id, timestamp, sdnn, rmssd, pnn50, ln_rmssd, stress, heart_rate, systolic_bp, diastolic_bp FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_hrv(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let records = body.get("records").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut count = 0i64;
    for r in &records {
        sqlx::query("INSERT INTO hrv (user_id, timestamp, rmssd, stress, heart_rate, systolic_bp, diastolic_bp) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(uid)
            .bind(r.get("timestamp").and_then(|v| v.as_str()).and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok()).unwrap_or_else(Utc::now))
            .bind(r.get("rmssd").and_then(|v| v.as_f64()).map(|v| v as f32))
            .bind(r.get("stress").and_then(|v| v.as_f64()).map(|v| v as f32))
            .bind(r.get("heartRate").and_then(|v| v.as_f64()).map(|v| v as f32))
            .bind(r.get("systolicBP").and_then(|v| v.as_f64()).map(|v| v as f32))
            .bind(r.get("diastolicBP").and_then(|v| v.as_f64()).map(|v| v as f32))
            .execute(&pool).await?;
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "hrv" } })))
}

// ═══ SpO2 ═══
pub async fn get_spo2(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, SpO2Record>(
        "SELECT id, NULL::text as user_id, timestamp, value, confidence FROM spo2 WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_spo2(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<BiometricUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO spo2 (user_id, timestamp, value) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32).execute(&pool).await?;
        count += 1;
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": count, "type": "spo2" } })))
}

// ═══ TEMPERATURE ═══
pub async fn get_temperature(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, TemperatureRecord>(
        "SELECT id, NULL::text as user_id, timestamp, value, location FROM temperature WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_temperature(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<BiometricUpload>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut count = 0i64;
    for r in &body.records {
        sqlx::query("INSERT INTO temperature (user_id, timestamp, value) VALUES ($1, $2, $3)")
            .bind(uid).bind(r.timestamp).bind(r.value as f32).execute(&pool).await?;
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
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, PPIRecord>(
        "SELECT id, NULL::text as user_id, timestamp, intervals, rmssd, coherence FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_ppi(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    sqlx::query("INSERT INTO ppi (user_id, timestamp, intervals, rmssd, coherence) VALUES ($1, NOW(), $2, $3, $4)")
        .bind(uid)
        .bind(body.get("intervals"))
        .bind(body.get("rmssd").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("coherence").and_then(|v| v.as_f64()).map(|v| v as f32))
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 1, "type": "ppi" } })))
}

// ═══ ECG ═══
pub async fn get_ecg(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, (uuid::Uuid, chrono::DateTime<Utc>, Option<i32>, Option<i32>, Option<serde_json::Value>)>(
        "SELECT id, timestamp, duration_seconds, sample_rate, analysis FROM ecg WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 50"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({
        "id": r.0, "timestamp": r.1, "durationSeconds": r.2, "sampleRate": r.3, "analysis": r.4
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn post_ecg(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO ecg (id, user_id, timestamp, duration_seconds, sample_rate, samples, analysis) VALUES ($1, $2, NOW(), $3, $4, $5, $6)")
        .bind(id).bind(uid)
        .bind(body.get("durationSeconds").and_then(|v| v.as_i64()).map(|v| v as i32))
        .bind(body.get("sampleRate").and_then(|v| v.as_i64()).map(|v| v as i32))
        .bind(body.get("samples"))
        .bind(body.get("analysis"))
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "id": id, "type": "ecg" } })))
}

// ═══ ACTIVITY ═══
pub async fn get_activity(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, ActivityRecord>(
        "SELECT id, NULL::text as user_id, timestamp, steps, calories, distance, active_minutes, mets, activity_type FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": rows })))
}

pub async fn post_activity(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    sqlx::query("INSERT INTO activity (user_id, timestamp, steps, calories, distance, active_minutes, mets, activity_type) VALUES ($1, NOW(), $2, $3, $4, $5, $6, $7)")
        .bind(uid)
        .bind(body.get("steps").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("calories").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("distance").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("activeMinutes").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("mets").and_then(|v| v.as_f64()).map(|v| v as f32))
        .bind(body.get("activityType").and_then(|v| v.as_str()))
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "recordsSaved": 1, "type": "activity" } })))
}

// ═══ DERIVED METRICS ═══
pub async fn get_blood_pressure(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>, Option<f32>)>(
        "SELECT timestamp, systolic_bp, diastolic_bp FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND systolic_bp IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "systolic": r.1, "diastolic": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_stress(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
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
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>)>(
        "SELECT timestamp, rmssd FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND rmssd IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "rmssd": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn get_coherence(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<TimeRangeQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(default_from);
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>)>(
        "SELECT timestamp, coherence FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND coherence IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "coherence": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
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
pub async fn sync(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<SyncRequest>) -> AppResult<Json<serde_json::Value>> {
    let uid = get_user_uuid(&pool, &user.privy_did).await?;
    let mut processed = 0i64;
    let received = body.records.len();

    for rec in &body.records {
        match rec.record_type.as_str() {
            "heart_rate" => {
                if let Some(bpm) = rec.data.get("bpm").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO heart_rate (user_id, timestamp, bpm) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(bpm as f32).execute(&pool).await?;
                    processed += 1;
                }
            }
            "hrv" => {
                sqlx::query("INSERT INTO hrv (user_id, timestamp, rmssd, stress, heart_rate, systolic_bp, diastolic_bp) VALUES ($1, $2, $3, $4, $5, $6, $7)")
                    .bind(uid).bind(rec.timestamp)
                    .bind(rec.data.get("rmssd").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("stress").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("heartRate").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("systolicBP").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("diastolicBP").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .execute(&pool).await?;
                processed += 1;
            }
            "spo2" => {
                if let Some(val) = rec.data.get("value").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO spo2 (user_id, timestamp, value) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(val as f32).execute(&pool).await?;
                    processed += 1;
                }
            }
            "temperature" => {
                if let Some(val) = rec.data.get("value").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO temperature (user_id, timestamp, value) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(val as f32).execute(&pool).await?;
                    processed += 1;
                }
            }
            "activity" => {
                sqlx::query("INSERT INTO activity (user_id, timestamp, steps, calories, active_minutes, mets) VALUES ($1, $2, $3, $4, $5, $6)")
                    .bind(uid).bind(rec.timestamp)
                    .bind(rec.data.get("steps").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("calories").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("activeMinutes").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .bind(rec.data.get("mets").and_then(|v| v.as_f64()).map(|v| v as f32))
                    .execute(&pool).await?;
                processed += 1;
            }
            _ => {}
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "syncId": uuid::Uuid::new_v4(), "recordsReceived": received, "recordsProcessed": processed, "deviceId": body.device_id }
    })))
}
