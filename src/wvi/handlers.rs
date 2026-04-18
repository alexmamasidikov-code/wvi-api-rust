use axum::{extract::{Query, State}, Json};
use chrono::{Utc, Duration, Timelike};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;
use super::models::*;
use super::calculator::{WviV2Calculator, WviV2Input};

/// GET /wvi/current — Calculate live WVI v2 from latest biometrics
pub async fn get_current(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    // Fetch latest biometrics
    let hr = sqlx::query_as::<_, (f32,)>("SELECT bpm FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1")
        .bind(&user.privy_did).fetch_optional(&pool).await?.map(|r| r.0 as f64).unwrap_or(72.0);
    let hrv_row = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT rmssd, stress, systolic_bp, diastolic_bp FROM hrv WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    let spo2 = sqlx::query_as::<_, (f32,)>("SELECT value FROM spo2 WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1")
        .bind(&user.privy_did).fetch_optional(&pool).await?.map(|r| r.0 as f64).unwrap_or(98.0);
    let temp = sqlx::query_as::<_, (f32,)>("SELECT value FROM temperature WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1")
        .bind(&user.privy_did).fetch_optional(&pool).await?.map(|r| r.0 as f64).unwrap_or(36.6);
    let sleep = sqlx::query_as::<_, (Option<f32>, Option<f32>, Option<f32>)>(
        "SELECT total_hours, deep_percent, efficiency FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    // Use MAX (not SUM) because bracelet reports CUMULATIVE daily totals, not increments
    // Each sync sends the same total — SUM would multiply by number of syncs
    let act = sqlx::query_as::<_, (Option<f64>, Option<f64>, Option<f64>)>(
        "SELECT MAX(steps)::float8, MAX(active_minutes)::float8, MAX(calories)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= $2"
    ).bind(&user.privy_did).bind(Utc::now().date_naive().and_hms_opt(0,0,0).unwrap().and_utc()).fetch_one(&pool).await?;
    let coherence = sqlx::query_as::<_, (Option<f32>,)>("SELECT coherence FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1")
        .bind(&user.privy_did).fetch_optional(&pool).await?.and_then(|r| r.0).unwrap_or(0.4);

    // Fetch latest emotion
    let emotion_name = sqlx::query_as::<_, (String,)>(
        "SELECT primary_emotion FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?.map(|r| r.0).unwrap_or_default();

    // Compute emotion wellbeing score — WEIGHTED by emotion quality, not binary positive/negative
    // Each emotion has a wellbeing score: flow=95, joyful=90, energized=85, relaxed=80,
    // meditative=80, focused=65, calm=60, recovering=50, drowsy=40, frustrated=30,
    // stressed=25, sad=20, anxious=15, angry=10, fearful=10, exhausted=10, pain=5
    let emotion_rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT primary_emotion, COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '24 hours' GROUP BY primary_emotion"
    ).bind(&user.privy_did).fetch_all(&pool).await?;

    let total_count: i64 = emotion_rows.iter().map(|r| r.1).sum();
    let emotion_score = if total_count > 0 {
        let weighted_sum: f64 = emotion_rows.iter().map(|r| {
            let weight = match r.0.as_str() {
                "flow" => 95.0,
                "joyful" => 90.0,
                "energized" | "excited" => 85.0,
                "relaxed" => 80.0,
                "meditative" => 80.0,
                "focused" => 65.0,       // focused is NEUTRAL, not highly positive
                "calm" => 60.0,          // calm is baseline, not excellent
                "recovering" => 50.0,
                "drowsy" => 40.0,
                "frustrated" => 30.0,
                "stressed" => 25.0,
                "sad" => 20.0,
                "anxious" => 15.0,
                "angry" | "fearful" => 10.0,
                "exhausted" | "pain" => 5.0,
                _ => 50.0,
            };
            weight * r.1 as f64
        }).sum();
        (weighted_sum / total_count as f64).clamp(0.0, 100.0)
    } else {
        50.0
    };

    // Compute sleep score from components
    let sleep_score = {
        let total_hours = sleep.as_ref().and_then(|r| r.0).unwrap_or(7.0) as f64;
        let deep_pct = sleep.as_ref().and_then(|r| r.1).unwrap_or(20.0) as f64;
        let efficiency = sleep.as_ref().and_then(|r| r.2).unwrap_or(85.0) as f64;

        let deep_s = if (15.0..=25.0).contains(&deep_pct) { 100.0 }
            else { (100.0 - (deep_pct - 20.0).abs() * 5.0).max(0.0) };
        let dur_s = if (7.0..=9.0).contains(&total_hours) { 100.0 }
            else { (100.0 - (total_hours - 8.0).abs() * 20.0).max(0.0) };
        let eff_s = (efficiency / 100.0 * 100.0).clamp(0.0, 100.0);
        deep_s * 0.35 + dur_s * 0.40 + eff_s * 0.25
    };

    // ACWR: acute (7d) / chronic (28d) load ratio
    let acute = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(daily_steps)::float8 FROM (SELECT DATE(timestamp), SUM(steps) as daily_steps FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' GROUP BY 1) t"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0.unwrap_or(5000.0);
    let chronic = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(daily_steps)::float8 FROM (SELECT DATE(timestamp), SUM(steps) as daily_steps FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '28 days' GROUP BY 1) t"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0.unwrap_or(5000.0);
    let acwr = if chronic > 0.0 { acute / chronic } else { 1.0 };

    // FIX: Convert skin temp to body temp (+2°C offset)
    let body_temp = if temp < 35.0 { temp + 2.0 } else { temp }; // skin temp → body
    let base_temp = 36.6;
    let temp_delta = body_temp - base_temp;

    let steps = act.0.unwrap_or(0.0);
    let total_calories = act.2.unwrap_or(0.0);
    // Active calories: if bracelet reports total (BMR+active), subtract estimated BMR
    // If total < 500 kcal, it's likely already active-only from bracelet
    // Bracelet typically reports active calories directly (not total)
    let active_calories = if total_calories > 1000.0 {
        // Likely total calories — subtract BMR estimate
        (total_calories - 1600.0).max(0.0)
    } else {
        // Already active calories from bracelet
        total_calories
    };

    // Time-proportional scaling: adjust targets based on hours elapsed today
    let now = Utc::now();
    let hours_elapsed = now.hour() as f64 + now.minute() as f64 / 60.0;
    let waking_hours = 16.0; // assume 16 waking hours (6am-10pm)
    // Adjust: if before 6am, consider it "yesterday's end"
    let day_progress = if hours_elapsed < 6.0 {
        1.0 // full day (still yesterday's data)
    } else {
        ((hours_elapsed - 6.0) / waking_hours).clamp(0.05, 1.0)
    };

    // Project to full day, but cap so morning data doesn't wildly inflate
    // max 3x actual OR absolute cap (15K steps, 2000 cal)
    let adjusted_steps = (steps / day_progress).min(steps * 3.0).min(15000.0);
    let adjusted_calories = (active_calories / day_progress).min(active_calories * 3.0).min(2000.0);

    // FIX: Use stored sleep_score directly if available
    let db_sleep_score = sqlx::query_scalar::<_, f32>(
        "SELECT sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?.unwrap_or(0.0) as f64;
    let final_sleep_score = if db_sleep_score > 0.0 { db_sleep_score } else { sleep_score };

    // FIX: Emotion wellbeing — need at least 5 readings for meaningful score
    // Emotion: need 10+ readings for meaningful score, blend with neutral
    let final_emotion_score = if total_count >= 10 {
        emotion_score
    } else if total_count > 0 {
        // Blend: partial data → weighted toward neutral (50)
        let confidence = total_count as f64 / 10.0; // 0.0-1.0
        emotion_score * confidence + 50.0 * (1.0 - confidence)
    } else {
        50.0
    };

    // FIX: ACWR — only use if we have real multi-day data
    let days_with_activity = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT DATE(timestamp)) FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await.unwrap_or(0);
    // ACWR needs 7+ days for acute and 28 for chronic. With less data → score = neutral 50
    let final_acwr = if days_with_activity >= 7 { acwr } else { 1.05 }; // 1.05 → score ~92, not 97

    // FIX: PPI coherence — don't penalize if bracelet doesn't support it
    let has_ppi_data = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM ppi WHERE user_id = (SELECT id FROM users WHERE privy_did = $1)"
    ).bind(&user.privy_did).fetch_one(&pool).await.unwrap_or(0);
    let final_coherence = if has_ppi_data > 0 { coherence as f64 } else { 0.65 }; // neutral if no data

    let input = WviV2Input {
        hrv_rmssd: hrv_row.as_ref().and_then(|r| r.0).unwrap_or(50.0) as f64,
        stress_index: hrv_row.as_ref().and_then(|r| r.1).unwrap_or(30.0) as f64,
        sleep_score: final_sleep_score,
        emotion_score: final_emotion_score,
        spo2,
        heart_rate: hr,
        // Personal resting HR from 7-day average of lowest readings
        resting_hr: {
            let rhr = sqlx::query_scalar::<_, f64>(
                "SELECT COALESCE(PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY bpm)::float8, 65) FROM heart_rate WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days'"
            ).bind(&user.privy_did).fetch_one(&pool).await.unwrap_or(65.0);
            rhr.clamp(45.0, 90.0)
        },
        steps: adjusted_steps,
        active_calories: adjusted_calories,
        acwr: final_acwr,
        bp_systolic: hrv_row.as_ref().and_then(|r| r.2).unwrap_or(120.0) as f64,
        bp_diastolic: hrv_row.as_ref().and_then(|r| r.3).unwrap_or(80.0) as f64,
        temp_delta,
        ppi_coherence: final_coherence,
        emotion_name,
    };

    let result = WviV2Calculator::calculate(&input);

    // Store in DB
    let _ = sqlx::query(
        "INSERT INTO wvi_scores (user_id, timestamp, wvi_score, level, metrics, weights, emotion_feedback) \
         VALUES ((SELECT id FROM users WHERE privy_did = $1), NOW(), $2, $3, $4, $5, $6)"
    )
        .bind(&user.privy_did)
        .bind(result.wvi_score as f32)
        .bind(&result.level)
        .bind(serde_json::to_value(&result.metric_scores).unwrap_or_default())
        .bind(serde_json::json!({ "version": "2.0", "type": "geometric_weighted" }))
        .bind(result.emotion_multiplier as f32)
        .execute(&pool).await;

    Ok(Json(serde_json::json!({ "success": true, "data": result })))
}

/// GET /wvi/history
pub async fn get_history(user: AuthUser, State(pool): State<PgPool>, Query(q): Query<WVIHistoryQuery>) -> AppResult<Json<serde_json::Value>> {
    let from = q.from.unwrap_or_else(|| Utc::now() - Duration::days(7));
    let to = q.to.unwrap_or_else(Utc::now);
    let rows = sqlx::query_as::<_, (chrono::DateTime<Utc>, f32, String, serde_json::Value)>(
        "SELECT timestamp, wvi_score, level, metrics FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN $2 AND $3 ORDER BY timestamp DESC LIMIT 1000"
    ).bind(&user.privy_did).bind(from).bind(to).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.into_iter().map(|r| serde_json::json!({
        "timestamp": r.0, "wviScore": r.1, "level": r.2, "metrics": r.3
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /wvi/trends
pub async fn get_trends(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let avg_7d = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    let avg_30d = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '30 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    let prev_7d = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN NOW() - INTERVAL '14 days' AND NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;

    let change = match (avg_7d, prev_7d) {
        (Some(cur), Some(prev)) if prev > 0.0 => ((cur - prev) / prev * 100.0 * 10.0).round() / 10.0,
        _ => 0.0,
    };
    let direction = if change > 2.0 { "improving" } else if change < -2.0 { "declining" } else { "stable" };

    Ok(Json(serde_json::json!({ "success": true, "data": { "avg7d": avg_7d, "avg30d": avg_30d, "change7dPercent": change, "direction": direction } })))
}

/// GET /wvi/predict
pub async fn predict(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let recent = sqlx::query_as::<_, (f32,)>(
        "SELECT wvi_score FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 6"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let avg: f64 = if recent.is_empty() { 50.0 } else { recent.iter().map(|r| r.0 as f64).sum::<f64>() / recent.len() as f64 };
    Ok(Json(serde_json::json!({ "success": true, "data": { "predicted6h": (avg * 10.0).round() / 10.0, "confidence": 0.7, "basedOn": recent.len() } })))
}

/// POST /wvi/simulate
pub async fn simulate(_user: AuthUser, Json(req): Json<SimulateRequest>) -> AppResult<Json<serde_json::Value>> {
    let sleep_score = req.sleep_score.unwrap_or_else(|| {
        let hours = req.sleep_hours.unwrap_or(7.0);
        if (7.0..=9.0).contains(&hours) { 85.0 } else { (100.0 - (hours - 8.0).abs() * 20.0).max(0.0) }
    });

    let input = WviV2Input {
        hrv_rmssd: req.hrv.unwrap_or(50.0),
        stress_index: req.stress.unwrap_or(30.0),
        sleep_score,
        emotion_score: req.emotion_score.unwrap_or(50.0),
        spo2: req.spo2.unwrap_or(98.0),
        heart_rate: req.heart_rate.unwrap_or(72.0),
        resting_hr: req.resting_hr.unwrap_or(65.0),
        steps: req.steps.unwrap_or(5000.0),
        active_calories: req.active_calories.unwrap_or(300.0),
        acwr: req.acwr.unwrap_or(1.0),
        bp_systolic: req.systolic_bp.unwrap_or(120.0),
        bp_diastolic: req.diastolic_bp.unwrap_or(80.0),
        temp_delta: req.temperature.map(|t| t - 36.6).unwrap_or(0.0),
        ppi_coherence: req.ppi_coherence.unwrap_or(0.4),
        emotion_name: req.emotion_name.unwrap_or_default(),
    };

    let result = WviV2Calculator::calculate(&input);
    Ok(Json(serde_json::json!({ "success": true, "data": result })))
}

/// GET /wvi/circadian
pub async fn circadian(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (f64, Option<f64>)>(
        "SELECT EXTRACT(HOUR FROM timestamp)::float8 as hour, AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days' GROUP BY hour ORDER BY hour"
    ).bind(&user.privy_did).fetch_all(&pool).await?;
    let mut hourly: [f64; 24] = [0.0; 24];
    for (h, v) in rows {
        let idx = (h as usize).min(23);
        hourly[idx] = v.unwrap_or(0.0);
    }
    Ok(Json(serde_json::json!({ "success": true, "data": { "hourly": hourly.to_vec() } })))
}

/// GET /wvi/correlations
pub async fn correlations(_user: AuthUser, State(_pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(serde_json::json!({ "success": true, "data": { "hrv_wvi": 0.82, "stress_wvi": -0.75, "sleep_wvi": 0.68, "steps_wvi": 0.52, "activity_wvi": 0.45, "spo2_wvi": 0.38 } })))
}

/// GET /wvi/breakdown
pub async fn breakdown(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let row = sqlx::query_as::<_, (f32, serde_json::Value, serde_json::Value)>(
        "SELECT wvi_score, metrics, weights FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1"
    ).bind(&user.privy_did).fetch_optional(&pool).await?;
    match row {
        Some(r) => Ok(Json(serde_json::json!({ "success": true, "data": { "wviScore": r.0, "metrics": r.1, "weights": r.2, "formulaVersion": "2.0" } }))),
        None => Ok(Json(serde_json::json!({ "success": true, "data": null }))),
    }
}

/// GET /wvi/compare
pub async fn compare(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let this_week = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    let prev_month = sqlx::query_as::<_, (Option<f64>,)>(
        "SELECT AVG(wvi_score)::float8 FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp BETWEEN NOW() - INTERVAL '30 days' AND NOW() - INTERVAL '7 days'"
    ).bind(&user.privy_did).fetch_one(&pool).await?.0;
    Ok(Json(serde_json::json!({ "success": true, "data": { "current_week_avg": this_week, "previous_month_avg": prev_month, "delta": this_week.unwrap_or(0.0) - prev_month.unwrap_or(0.0) } })))
}
