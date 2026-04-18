use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn last_night(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT date, total_hours, sleep_score, efficiency, deep_percent, light_percent, rem_percent, avg_hr, avg_hrv FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "date": r.0, "totalHours": r.1, "sleepScore": r.2, "efficiency": r.3, "deepPercent": r.4, "lightPercent": r.5, "remPercent": r.6, "avgHR": r.7, "avgHRV": r.8 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

pub async fn score_history(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>)>(
        "SELECT date, sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 30"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "date": r.0, "score": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn architecture(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT deep_percent, light_percent, rem_percent, awake_percent FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "deep": r.0, "light": r.1, "rem": r.2, "awake": r.3 } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

pub async fn consistency(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<f32>,)>(
        "SELECT total_hours FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 14"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let hours: Vec<f64> = rows.iter().filter_map(|r| r.0.map(|v| v as f64)).collect();
    let days: Vec<i32> = hours.iter().map(|h| (h * 60.0).round() as i32).collect();
    let avg = if hours.is_empty() { 0.0 } else { hours.iter().sum::<f64>() / hours.len() as f64 };
    let variance = if hours.len() > 1 { hours.iter().map(|h| (h - avg).powi(2)).sum::<f64>() / hours.len() as f64 } else { 0.0 };
    let consistency_score = (100.0 - variance * 10.0).clamp(0.0, 100.0);
    let stddev_min = (variance.sqrt() * 60.0 * 10.0).round() / 10.0;
    Ok(Json(serde_json::json!({ "success": true, "data": { "consistency_pct": (consistency_score * 10.0).round() / 10.0, "stddev_min": stddev_min, "days": days, "avgHours": (avg * 10.0).round() / 10.0 } })))
}

pub async fn debt(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<f32>,)>(
        "SELECT total_hours FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 7"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let total: f64 = rows.iter().filter_map(|r| r.0.map(|v| v as f64)).sum();
    let target = 8.0 * rows.len() as f64;
    let debt_hours = (target - total).max(0.0);
    Ok(Json(serde_json::json!({ "success": true, "data": { "debtHours": (debt_hours * 10.0).round() / 10.0, "daysTracked": rows.len(), "targetPerNight": 8.0 } })))
}

pub async fn phases(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (chrono::NaiveDate, Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT date, deep_percent, light_percent, rem_percent, awake_percent FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 7"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({ "date": r.0, "deep": r.1, "light": r.2, "rem": r.3, "awake": r.4 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

pub async fn optimal_window(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>, Option<chrono::DateTime<chrono::Utc>>, Option<f32>)>(
        "SELECT bedtime, wake_time, total_hours FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND bedtime IS NOT NULL ORDER BY date DESC LIMIT 14"
    ).bind(&user.privy_did).fetch_all(&pool).await?;

    if rows.is_empty() {
        return Ok(Json(serde_json::json!({
            "success": true,
            "data": { "bedtime": null, "wake": null, "wind_down": null, "target_minutes": 480, "reasoning": "Not enough sleep history yet — log a few nights to get a personalized window." }
        })));
    }

    // Average bedtime as minutes-from-18:00 to handle the midnight wrap.
    let mut bed_mins: Vec<i32> = Vec::with_capacity(rows.len());
    let mut wake_mins: Vec<i32> = Vec::with_capacity(rows.len());
    let mut durations: Vec<f64> = Vec::with_capacity(rows.len());
    for (b, w, d) in &rows {
        if let Some(b) = b {
            let local = b.with_timezone(&chrono::Local);
            use chrono::Timelike;
            let mut m = local.hour() as i32 * 60 + local.minute() as i32;
            if m < 18 * 60 { m += 24 * 60; }
            bed_mins.push(m);
        }
        if let Some(w) = w {
            use chrono::Timelike;
            let local = w.with_timezone(&chrono::Local);
            wake_mins.push(local.hour() as i32 * 60 + local.minute() as i32);
        }
        if let Some(d) = d { durations.push(*d as f64); }
    }

    let avg_bed = bed_mins.iter().sum::<i32>() as f64 / bed_mins.len().max(1) as f64;
    let avg_wake = if !wake_mins.is_empty() { wake_mins.iter().sum::<i32>() as f64 / wake_mins.len() as f64 } else { 6.5 * 60.0 };
    let avg_dur = if !durations.is_empty() { durations.iter().sum::<f64>() / durations.len() as f64 } else { 8.0 };

    let shift = if avg_dur < 7.0 { -15.0 } else { 0.0 };
    let rec_bed = (avg_bed + shift).rem_euclid(24.0 * 60.0);
    let wind_down = (rec_bed - 30.0).rem_euclid(24.0 * 60.0);
    let fmt = |m: f64| { let mm = m.round() as i32; format!("{:02}:{:02}", (mm / 60) % 24, mm % 60) };
    let bedtime_s = fmt(rec_bed);
    let wake_s = fmt(avg_wake);
    let wind_s = fmt(wind_down);
    let target_min = (avg_dur.max(7.5) * 60.0).round() as i32;
    let reasoning = format!(
        "Based on your {}-night average, your body winds down around {}. Aim for bed by {} to hit {:.1}h with your typical {} wake.",
        rows.len(), wind_s, bedtime_s, (target_min as f64) / 60.0, wake_s
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "data": { "bedtime": bedtime_s, "wake": wake_s, "wind_down": wind_s, "target_minutes": target_min, "reasoning": reasoning }
    })))
}
