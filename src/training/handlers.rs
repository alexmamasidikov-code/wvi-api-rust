use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn recommendation(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hr = sqlx::query_scalar::<_, f32>("SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?.unwrap_or(70.0);
    let hrv = sqlx::query_scalar::<_, f32>("SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1")
        .bind(uid).fetch_optional(&pool).await?.unwrap_or(50.0);
    let steps = sqlx::query_scalar::<_, i64>("SELECT COALESCE(SUM(steps)::bigint, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '24 hours'")
        .bind(uid).fetch_one(&pool).await.unwrap_or(0);

    let recovery_level = if hrv > 60.0 { "high" } else if hrv > 30.0 { "moderate" } else { "low" };

    let (activity_type, duration, intensity) = match recovery_level {
        "high" => ("HIIT or strength training", "30-45 min", "high"),
        "moderate" => ("Moderate cardio (jogging, cycling)", "20-30 min", "moderate"),
        _ => ("Light activity (walking, yoga, stretching)", "15-20 min", "low"),
    };

    let steps_goal = 10000 - steps.min(10000);

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "recommendation": activity_type,
            "duration": duration,
            "intensity": intensity,
            "recoveryLevel": recovery_level,
            "currentHRV": hrv,
            "restingHR": hr,
            "stepsRemaining": steps_goal,
            "note": format!("Based on HRV {:.0}ms and resting HR {:.0}bpm", hrv, hr),
        }
    })))
}

pub async fn weekly_plan(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let avg_hrv = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(AVG(rmssd)::float8, 50) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(50.0);

    let avg_steps = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(AVG(steps)::float8, 0) FROM activity WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '7 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(0.0);

    let fitness_level = if avg_hrv > 60.0 && avg_steps > 8000.0 { "advanced" }
        else if avg_hrv > 30.0 || avg_steps > 5000.0 { "intermediate" }
        else { "beginner" };

    let plan = match fitness_level {
        "advanced" => serde_json::json!([
            {"day": "Monday",    "workout": "HIIT",           "duration": "40 min", "intensity": "high"},
            {"day": "Tuesday",   "workout": "Strength",       "duration": "45 min", "intensity": "high"},
            {"day": "Wednesday", "workout": "Active recovery","duration": "30 min", "intensity": "low"},
            {"day": "Thursday",  "workout": "Tempo run",      "duration": "35 min", "intensity": "moderate"},
            {"day": "Friday",    "workout": "Strength",       "duration": "45 min", "intensity": "high"},
            {"day": "Saturday",  "workout": "Long run",       "duration": "60 min", "intensity": "moderate"},
            {"day": "Sunday",    "workout": "Rest",           "duration": "—",      "intensity": "none"},
        ]),
        "intermediate" => serde_json::json!([
            {"day": "Monday",    "workout": "Jogging",        "duration": "30 min", "intensity": "moderate"},
            {"day": "Tuesday",   "workout": "Strength",       "duration": "30 min", "intensity": "moderate"},
            {"day": "Wednesday", "workout": "Walking",        "duration": "30 min", "intensity": "low"},
            {"day": "Thursday",  "workout": "Cycling",        "duration": "30 min", "intensity": "moderate"},
            {"day": "Friday",    "workout": "Strength",       "duration": "30 min", "intensity": "moderate"},
            {"day": "Saturday",  "workout": "Outdoor walk",   "duration": "45 min", "intensity": "low"},
            {"day": "Sunday",    "workout": "Rest",           "duration": "—",      "intensity": "none"},
        ]),
        _ => serde_json::json!([
            {"day": "Monday",    "workout": "Walking",        "duration": "20 min", "intensity": "low"},
            {"day": "Tuesday",   "workout": "Stretching",     "duration": "15 min", "intensity": "low"},
            {"day": "Wednesday", "workout": "Walking",        "duration": "20 min", "intensity": "low"},
            {"day": "Thursday",  "workout": "Yoga",           "duration": "20 min", "intensity": "low"},
            {"day": "Friday",    "workout": "Walking",        "duration": "20 min", "intensity": "low"},
            {"day": "Saturday",  "workout": "Light activity", "duration": "30 min", "intensity": "low"},
            {"day": "Sunday",    "workout": "Rest",           "duration": "—",      "intensity": "none"},
        ]),
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "fitnessLevel": fitness_level,
            "avgHRV": (avg_hrv * 10.0).round() / 10.0,
            "avgDailySteps": avg_steps.round() as i64,
            "plan": plan,
        }
    })))
}

pub async fn overtraining_risk(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hrv_now = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(AVG(rmssd)::float8, 50) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '3 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(50.0);

    let hrv_baseline = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(AVG(rmssd)::float8, 50) FROM hrv WHERE user_id = $1 AND timestamp >= NOW() - INTERVAL '30 days' AND timestamp < NOW() - INTERVAL '3 days'"
    ).bind(uid).fetch_one(&pool).await.unwrap_or(50.0);

    let resting_hr = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(70.0);

    let hrv_drop_pct = if hrv_baseline > 0.0 { (hrv_baseline - hrv_now) / hrv_baseline * 100.0 } else { 0.0 };

    let risk_level = if hrv_drop_pct > 20.0 || resting_hr > 90.0 { "high" }
        else if hrv_drop_pct > 10.0 || resting_hr > 80.0 { "moderate" }
        else { "low" };

    let recommendation = match risk_level {
        "high" => "Take 2-3 rest days. Prioritize sleep and nutrition.",
        "moderate" => "Reduce training intensity by 30%. Focus on recovery.",
        _ => "Training load is appropriate. Continue current plan.",
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "riskLevel": risk_level,
            "hrvDrop": (hrv_drop_pct * 10.0).round() / 10.0,
            "hrvNow": (hrv_now * 10.0).round() / 10.0,
            "hrvBaseline": (hrv_baseline * 10.0).round() / 10.0,
            "restingHR": resting_hr,
            "recommendation": recommendation,
        }
    })))
}

pub async fn optimal_time(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;

    let hrv = sqlx::query_scalar::<_, f32>(
        "SELECT rmssd FROM hrv WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(50.0);

    let resting_hr = sqlx::query_scalar::<_, f32>(
        "SELECT bpm FROM heart_rate WHERE user_id = $1 ORDER BY timestamp DESC LIMIT 1"
    ).bind(uid).fetch_optional(&pool).await?.unwrap_or(70.0);

    // Simple heuristic: high HRV + normal HR => morning workout optimal
    // Low HRV => afternoon better (body needs morning recovery)
    let (optimal_window, rationale) = if hrv > 60.0 && resting_hr < 75.0 {
        ("06:00 – 09:00", "High HRV and normal resting HR indicate peak morning readiness.")
    } else if hrv > 30.0 {
        ("15:00 – 18:00", "Moderate recovery — afternoon allows further physiological priming.")
    } else {
        ("18:00 – 20:00", "Low HRV suggests delayed readiness; light evening activity recommended.")
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "optimalWindow": optimal_window,
            "rationale": rationale,
            "currentHRV": hrv,
            "restingHR": resting_hr,
        }
    })))
}
