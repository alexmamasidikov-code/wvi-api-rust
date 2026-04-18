use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

async fn fetch_latest_biometrics(pool: &PgPool, uid: uuid::Uuid) -> AppResult<(f32, f32, f32)> {
    let hr = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(pool).await?.unwrap_or(0.0);

    let spo2 = sqlx::query_scalar::<_, f32>(
        "SELECT value FROM spo2 WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(pool).await?.unwrap_or(0.0);

    let hrv = sqlx::query_scalar::<_, Option<f32>>(
        "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(pool).await?.flatten().unwrap_or(0.0);

    Ok((hr, spo2, hrv))
}

pub async fn assessment(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;
    let (hr, spo2, hrv) = fetch_latest_biometrics(&pool, uid).await?;

    let mut risk_score = 0.0f64;
    let mut flags: Vec<&str> = vec![];

    if hr > 100.0 { risk_score += 20.0; flags.push("Elevated heart rate"); }
    if hr > 0.0 && hr < 50.0 { risk_score += 15.0; flags.push("Low heart rate"); }
    if spo2 > 0.0 && spo2 < 95.0 { risk_score += 25.0; flags.push("Low SpO2"); }
    if spo2 > 0.0 && spo2 < 90.0 { risk_score += 30.0; flags.push("Critical SpO2"); }
    if hrv > 0.0 && hrv < 20.0 { risk_score += 15.0; flags.push("Very low HRV — high stress"); }

    let level = if risk_score < 10.0 { "low" } else if risk_score < 30.0 { "moderate" } else if risk_score < 60.0 { "elevated" } else { "high" };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "riskScore": risk_score,
            "level": level,
            "flags": flags,
            "heartRate": hr,
            "spo2": spo2,
            "hrv": hrv
        }
    })))
}

pub async fn anomalies(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    // Detect HR outliers: values > 2 std deviations from mean
    let hr_outliers = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, bpm FROM heart_rate WHERE user_id = $1 AND ABS(bpm - (SELECT AVG(bpm) FROM heart_rate WHERE user_id = $1)) > 2 * GREATEST((SELECT STDDEV(bpm) FROM heart_rate WHERE user_id = $1), 1) ORDER BY timestamp DESC LIMIT 20"
    ).bind(uid).bind(uid).bind(uid).fetch_all(&pool).await.unwrap_or_default();

    let spo2_low = sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, f32)>(
        "SELECT timestamp, value FROM spo2 WHERE user_id = $1 AND value < 95.0 ORDER BY timestamp DESC LIMIT 20"
    ).bind(uid).fetch_all(&pool).await.unwrap_or_default();

    let hr_anomalies: Vec<serde_json::Value> = hr_outliers.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "type": "heart_rate_outlier", "value": r.1
    })).collect();

    let spo2_anomalies: Vec<serde_json::Value> = spo2_low.iter().map(|r| serde_json::json!({
        "timestamp": r.0, "type": "low_spo2", "value": r.1
    })).collect();

    let mut all_anomalies = hr_anomalies;
    all_anomalies.extend(spo2_anomalies);

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "anomalies": all_anomalies,
            "count": all_anomalies.len()
        }
    })))
}

pub async fn chronic_flags(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let avg_hr = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(bpm)::float8 FROM heart_rate WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '30 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let avg_spo2 = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(value)::float8 FROM spo2 WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '30 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let avg_hrv = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(rmssd)::float8 FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '30 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let avg_sleep = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT AVG(total_hours)::float8 FROM sleep_records WHERE user_id = $1 AND date >= NOW()::date - 30"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let mut flags: Vec<serde_json::Value> = vec![];

    if avg_hr > 85.0 {
        flags.push(serde_json::json!({ "label": "Chronically elevated resting HR", "value": avg_hr, "threshold": 85.0, "severity": "moderate" }));
    }
    if avg_spo2 > 0.0 && avg_spo2 < 96.0 {
        flags.push(serde_json::json!({ "label": "Persistently low SpO2", "value": avg_spo2, "threshold": 96.0, "severity": "elevated" }));
    }
    if avg_hrv > 0.0 && avg_hrv < 30.0 {
        flags.push(serde_json::json!({ "label": "Chronically low HRV", "value": avg_hrv, "threshold": 30.0, "severity": "moderate" }));
    }
    if avg_sleep > 0.0 && avg_sleep < 6.5 {
        flags.push(serde_json::json!({ "label": "Chronic sleep deprivation", "value": avg_sleep, "threshold": 6.5, "severity": "elevated" }));
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "flags": flags,
            "period": "30d",
            "avgHeartRate": (avg_hr * 10.0).round() / 10.0,
            "avgSpO2": (avg_spo2 * 10.0).round() / 10.0,
            "avgHRV": (avg_hrv * 10.0).round() / 10.0,
            "avgSleepHours": (avg_sleep * 10.0).round() / 10.0
        }
    })))
}

pub async fn correlations(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    // Steps vs sleep score correlation (simple: days where both exist)
    let paired = sqlx::query_as::<_, (Option<f64>, Option<f64>)>(
        "SELECT sr.sleep_score::float8, COALESCE((SELECT SUM(steps) FROM activity WHERE user_id = $1 AND timestamp::date = sr.date), 0)::float8 FROM sleep_records sr WHERE sr.user_id = $1 ORDER BY sr.date DESC LIMIT 30"
    ).bind(uid).fetch_all(&pool).await.unwrap_or_default();

    let n = paired.len() as f64;
    let correlations_data = if n > 2.0 {
        let sleep_vals: Vec<f64> = paired.iter().filter_map(|r| r.0).collect();
        let steps_vals: Vec<f64> = paired.iter().filter_map(|r| r.1).collect();
        let len = sleep_vals.len().min(steps_vals.len()) as f64;
        if len > 1.0 {
            let s_mean = sleep_vals.iter().sum::<f64>() / len;
            let a_mean = steps_vals.iter().sum::<f64>() / len;
            let cov: f64 = sleep_vals.iter().zip(steps_vals.iter()).map(|(s, a)| (s - s_mean) * (a - a_mean)).sum::<f64>() / len;
            let s_std = (sleep_vals.iter().map(|v| (v - s_mean).powi(2)).sum::<f64>() / len).sqrt();
            let a_std = (steps_vals.iter().map(|v| (v - a_mean).powi(2)).sum::<f64>() / len).sqrt();
            let corr = if s_std > 0.0 && a_std > 0.0 { cov / (s_std * a_std) } else { 0.0 };
            vec![serde_json::json!({ "pair": "sleep_score_vs_steps", "correlation": (corr * 100.0).round() / 100.0, "dataPoints": len as i64 })]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "correlations": correlations_data,
            "note": "Pearson correlation coefficients, range -1 to 1"
        }
    })))
}

pub async fn volatility(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hr_std = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT STDDEV(bpm)::float8 FROM heart_rate WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let spo2_std = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT STDDEV(value)::float8 FROM spo2 WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let hrv_std = sqlx::query_scalar::<_, Option<f64>>(
        "SELECT STDDEV(rmssd)::float8 FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_optional(&pool).await?.flatten().unwrap_or(0.0);

    let hr_vol_level = if hr_std < 5.0 { "stable" } else if hr_std < 15.0 { "moderate" } else { "high" };
    let spo2_vol_level = if spo2_std < 1.0 { "stable" } else if spo2_std < 3.0 { "moderate" } else { "high" };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "period": "7d",
            "heartRate": { "stddev": (hr_std * 100.0).round() / 100.0, "volatility": hr_vol_level },
            "spo2": { "stddev": (spo2_std * 100.0).round() / 100.0, "volatility": spo2_vol_level },
            "hrv": { "stddev": (hrv_std * 100.0).round() / 100.0 }
        }
    })))
}
