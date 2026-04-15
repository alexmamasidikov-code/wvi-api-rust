use axum::{extract::{Query, State}, Json};
use chrono::{Utc, Duration};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use crate::emotions::engine::EmotionEngine;
use super::models::*;

fn default_from() -> chrono::DateTime<Utc> {
    Utc::now() - Duration::days(7)
}

fn default_to() -> chrono::DateTime<Utc> {
    // Add 1 day buffer for timezone differences
    Utc::now() + Duration::days(1)
}

pub async fn get_user_uuid(pool: &PgPool, privy_did: &str) -> AppResult<uuid::Uuid> {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM users WHERE privy_did = $1")
        .bind(privy_did)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))
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
    let to = q.to.unwrap_or_else(default_to);
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
    let to = q.to.unwrap_or_else(default_to);
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
    let to = q.to.unwrap_or_else(default_to);
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
    let to = q.to.unwrap_or_else(default_to);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, Option<f32>, Option<f32>)>(
        "SELECT timestamp, systolic_bp, diastolic_bp FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND systolic_bp IS NOT NULL AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 500"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "timestamp": r.0, "systolic": r.1, "diastolic": r.2 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
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

// ═══ COMPUTED METRICS ═══
pub async fn get_computed(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    use super::computed;
    let uid = get_user_uuid(&pool, &user.privy_did).await?;

    let hr = sqlx::query_scalar::<_, f32>("SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?.unwrap_or(70.0) as f64;
    let hrv = sqlx::query_scalar::<_, f32>("SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?.unwrap_or(50.0) as f64;
    let spo2 = sqlx::query_scalar::<_, f32>("SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?.unwrap_or(98.0) as f64;
    let steps = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(steps)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'")
        .bind(uid).fetch_one(&pool).await? as f64;
    let active_min = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(active_minutes)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'")
        .bind(uid).fetch_one(&pool).await? as f64;

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
        75.0
    };

    let age = 30.0; // Default — should come from user profile
    let (sys, dia) = computed::estimate_blood_pressure(hr, hrv);
    let vo2_max = computed::estimate_vo2_max(hr, age);
    let coherence = computed::compute_coherence(hrv);
    let bio_age = computed::compute_bio_age(age, hr, hrv, spo2, steps, sleep_score);
    let training_load = computed::compute_training_load(active_min, hr, 220.0 - age);

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "bloodPressure": { "systolic": sys, "diastolic": dia },
            "vo2Max": (vo2_max * 10.0).round() / 10.0,
            "coherence": coherence.round(),
            "bioAge": bio_age,
            "trainingLoad": training_load,
            "sleepScore": sleep_score,
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
pub async fn sync(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<SyncRequest>) -> AppResult<Json<serde_json::Value>> {
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

    for rec in &body.records {
        match rec.record_type.as_str() {
            "heart_rate" => {
                if let Some(bpm) = rec.data.get("bpm").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO heart_rate (user_id, timestamp, bpm) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(bpm as f32).execute(&pool).await?;
                    latest_hr = Some(bpm);
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
                if let Some(v) = rec.data.get("rmssd").and_then(|v| v.as_f64()) { latest_hrv = Some(v); }
                if let Some(v) = rec.data.get("stress").and_then(|v| v.as_f64()) { latest_stress = Some(v); }
                if let Some(v) = rec.data.get("systolicBP").and_then(|v| v.as_f64()) { latest_systolic_bp = Some(v); }
                if let Some(v) = rec.data.get("heartRate").and_then(|v| v.as_f64()) {
                    if latest_hr.is_none() { latest_hr = Some(v); }
                }
                processed += 1;
            }
            "spo2" => {
                if let Some(val) = rec.data.get("value").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO spo2 (user_id, timestamp, value) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(val as f32).execute(&pool).await?;
                    latest_spo2 = Some(val);
                    processed += 1;
                }
            }
            "temperature" => {
                if let Some(val) = rec.data.get("value").and_then(|v| v.as_f64()) {
                    sqlx::query("INSERT INTO temperature (user_id, timestamp, value) VALUES ($1, $2, $3)")
                        .bind(uid).bind(rec.timestamp).bind(val as f32).execute(&pool).await?;
                    latest_temp = Some(val);
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
                if let Some(v) = rec.data.get("steps").and_then(|v| v.as_f64()) { latest_steps = Some(v); }
                processed += 1;
            }
            "ppi" => {
                if let Some(v) = rec.data.get("rmssd").and_then(|v| v.as_f64()) { latest_ppi_rmssd = Some(v); }
                if let Some(v) = rec.data.get("coherence").and_then(|v| v.as_f64()) { latest_ppi_coherence = Some(v); }
            }
            "sleep" => {
                if let Some(v) = rec.data.get("sleepScore").and_then(|v| v.as_f64()) { latest_sleep_score = Some(v); }
            }
            _ => {}
        }
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

        if let Err(e) = emotion_result {
            tracing::warn!("Emotion detection failed after sync (non-fatal): {}", e);
        }
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "syncId": uuid::Uuid::new_v4(), "recordsReceived": received, "recordsProcessed": processed, "deviceId": body.device_id }
    })))
}
