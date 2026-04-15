use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn csv_export(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hr_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let spo2_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let hrv_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, Option<f32>)>(
        "SELECT timestamp, rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let mut csv = String::from("timestamp,type,value\n");
    for r in &hr_rows { csv.push_str(&format!("{},heart_rate,{}\n", r.0.to_rfc3339(), r.1)); }
    for r in &spo2_rows { csv.push_str(&format!("{},spo2,{}\n", r.0.to_rfc3339(), r.1)); }
    for r in &hrv_rows {
        if let Some(v) = r.1 {
            csv.push_str(&format!("{},hrv_rmssd,{}\n", r.0.to_rfc3339(), v));
        }
    }

    let total = hr_rows.len() + spo2_rows.len() + hrv_rows.len();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "format": "csv",
            "content": csv,
            "records": total
        }
    })))
}

pub async fn json_export(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hr_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let spo2_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let hrv_rows = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, Option<f32>, Option<f32>)>(
        "SELECT timestamp, rmssd, stress FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(uid).fetch_all(&pool).await?;

    let heart_rate: Vec<serde_json::Value> = hr_rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "bpm": r.1
    })).collect();

    let spo2: Vec<serde_json::Value> = spo2_rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "value": r.1
    })).collect();

    let hrv: Vec<serde_json::Value> = hrv_rows.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "rmssd": r.1, "stress": r.2
    })).collect();

    let total = heart_rate.len() + spo2.len() + hrv.len();
    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "format": "json",
            "heartRate": heart_rate,
            "spo2": spo2,
            "hrv": hrv,
            "records": total
        }
    })))
}

pub async fn health_summary(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hr_latest = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(0.0);

    let hr_avg = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(bpm)::float8 FROM heart_rate WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let spo2_latest = sqlx::query_scalar::<_, f32>(
        "SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(0.0);

    let spo2_avg = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(value)::float8 FROM spo2 WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let hrv_latest = sqlx::query_scalar::<_, Option<f32>>(
        "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let hrv_avg = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(rmssd)::float8 FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let steps_today = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT SUM(steps)::float8 FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let sleep_score = sqlx::query_scalar::<_, Option<f32>>(
        "SELECT sleep_score FROM sleep_records WHERE user_id = $1 ORDER BY date DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    // Fetch latest WVI score
    let wvi_score = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT wvi_score::float8 FROM wvi_scores WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "generatedAt": chrono::Utc::now(),
            "heartRate": { "latest": hr_latest, "avg7d": (hr_avg * 10.0).round() / 10.0 },
            "spo2": { "latest": spo2_latest, "avg7d": (spo2_avg * 10.0).round() / 10.0 },
            "hrv": { "latest": hrv_latest, "avg7d": (hrv_avg * 10.0).round() / 10.0 },
            "stepsToday": steps_today,
            "sleepScore": sleep_score,
            "wviScore": (wvi_score * 10.0).round() / 10.0
        }
    })))
}
