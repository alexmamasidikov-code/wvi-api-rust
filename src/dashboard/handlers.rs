use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

pub async fn widgets(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let wvi = sqlx::query_as::<_, (Option<f32>,)>("SELECT wvi_score FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?.and_then(|r| r.0);
    let emotion = sqlx::query_as::<_, (String,)>("SELECT primary_emotion FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?.map(|r| r.0);
    let steps = sqlx::query_as::<_, (Option<f64>,)>("SELECT SUM(steps)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= CURRENT_DATE").bind(&user.privy_did).fetch_one(&pool).await?.0;
    let sleep = sqlx::query_as::<_, (Option<f32>,)>("SELECT sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?.and_then(|r| r.0);
    let active_alerts = sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM alerts WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND acknowledged = false").bind(&user.privy_did).fetch_one(&pool).await?.0;

    Ok(Json(serde_json::json!({ "success": true, "data": { "wvi": wvi, "emotion": emotion, "steps": steps, "sleepScore": sleep, "activeAlerts": active_alerts } })))
}

pub async fn daily_brief(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let wvi = sqlx::query_as::<_, (Option<f32>, Option<String>)>("SELECT wvi_score, level FROM wvi_scores WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY timestamp DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;
    let sleep = sqlx::query_as::<_, (Option<f32>, Option<f32>)>("SELECT total_hours, sleep_score FROM sleep_records WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) ORDER BY date DESC LIMIT 1").bind(&user.privy_did).fetch_optional(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "wvi": wvi.as_ref().and_then(|r| r.0), "level": wvi.as_ref().and_then(|r| r.1.clone()), "sleepHours": sleep.as_ref().and_then(|r| r.0), "sleepScore": sleep.as_ref().and_then(|r| r.1), "date": Utc::now().date_naive() } })))
}

pub async fn evening_review(user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let steps = sqlx::query_as::<_, (Option<f64>,)>("SELECT SUM(steps)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= CURRENT_DATE").bind(&user.privy_did).fetch_one(&pool).await?.0;
    let calories = sqlx::query_as::<_, (Option<f64>,)>("SELECT SUM(calories)::float8 FROM activity WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= CURRENT_DATE").bind(&user.privy_did).fetch_one(&pool).await?.0;
    let emotions = sqlx::query_as::<_, (String, i64)>("SELECT primary_emotion, COUNT(*) FROM emotions WHERE user_id = (SELECT id FROM users WHERE privy_did = $1) AND timestamp >= CURRENT_DATE GROUP BY 1 ORDER BY 2 DESC LIMIT 3").bind(&user.privy_did).fetch_all(&pool).await?;
    let top_emotions: Vec<serde_json::Value> = emotions.into_iter().map(|r| serde_json::json!({ "emotion": r.0, "count": r.1 })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": { "steps": steps, "calories": calories, "topEmotions": top_emotions, "date": Utc::now().date_naive() } })))
}
