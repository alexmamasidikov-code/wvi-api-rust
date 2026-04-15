use axum::{extract::State, Json};
use chrono::Utc;
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// GET /api/v1/social/feed
pub async fn get_feed(_user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (i64, String, i32, i32, chrono::DateTime<Utc>)>(
        "SELECT id, content, likes, comments, created_at FROM social_posts ORDER BY created_at DESC LIMIT 50"
    ).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "id": r.0, "content": r.1, "likes": r.2, "comments": r.3, "createdAt": r.4
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// POST /api/v1/social/post
pub async fn create_post(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    let uid = crate::biometrics::handlers::get_user_uuid(&pool, &user.privy_did).await?;
    let content = body.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
    sqlx::query("INSERT INTO social_posts (user_id, content) VALUES ($1, $2)")
        .bind(uid).bind(&content).execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "message": "Post created" } })))
}

/// GET /api/v1/social/challenges
pub async fn get_challenges(_user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (i64, String, Option<String>, Option<f32>, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>)>(
        "SELECT id, title, description, target_value, start_date, end_date FROM challenges ORDER BY created_at DESC"
    ).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "id": r.0, "title": r.1, "description": r.2, "targetValue": r.3, "startDate": r.4, "endDate": r.5
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}

/// GET /api/v1/social/leaderboard
pub async fn get_leaderboard(_user: AuthUser, State(pool): State<PgPool>) -> AppResult<Json<serde_json::Value>> {
    let rows = sqlx::query_as::<_, (Option<String>, Option<f32>)>(
        "SELECT u.email, ws.wvi_score FROM wvi_scores ws JOIN users u ON ws.user_id = u.id ORDER BY ws.wvi_score DESC LIMIT 20"
    ).fetch_all(&pool).await?;
    let data: Vec<serde_json::Value> = rows.iter().enumerate().map(|(i, r)| serde_json::json!({
        "rank": i + 1, "name": r.0.as_deref().unwrap_or("User"), "wviScore": r.1
    })).collect();
    Ok(Json(serde_json::json!({ "success": true, "data": data })))
}
