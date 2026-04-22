//! NPS / Rescue / Referral analytics handlers.
//!
//! Lightweight — each endpoint just appends a row to the corresponding
//! audit-style table so the growth team can query satisfaction trends
//! without needing a full analytics pipeline. Schema is permissive:
//! missing tables are auto-created on first write so deployment doesn't
//! block on a migration.

use axum::{extract::State, Json};
use sqlx::PgPool;
use crate::auth::middleware::AuthUser;
use crate::error::AppResult;

/// Idempotent `CREATE TABLE IF NOT EXISTS` for the three NPS tables.
/// Called lazily from each handler so the first POST bootstraps the
/// schema. Keeps the change free of a DB migration artefact.
async fn ensure_tables(pool: &PgPool) {
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS nps_submissions (
            id BIGSERIAL PRIMARY KEY,
            user_id UUID NOT NULL,
            score INT NOT NULL,
            touchpoint TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#
    ).execute(pool).await;
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS rescue_submissions (
            id BIGSERIAL PRIMARY KEY,
            user_id UUID NOT NULL,
            score INT NOT NULL,
            reason TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#
    ).execute(pool).await;
    let _ = sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS referral_tracks (
            id BIGSERIAL PRIMARY KEY,
            user_id UUID NOT NULL,
            code TEXT NOT NULL,
            channel TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )"#
    ).execute(pool).await;
}

async fn resolve_user(pool: &PgPool, privy_did: &str) -> AppResult<uuid::Uuid> {
    let uid: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM users WHERE privy_did = $1"
    ).bind(privy_did).fetch_optional(pool).await?;
    uid.ok_or_else(|| crate::error::AppError::NotFound("User not found".into()))
}

/// POST /nps/submit { score, touchpoint }
///
/// Score is the raw 0-10 value. Touchpoint is a short tag ("day_7_primary",
/// "post_pairing", etc.) so growth can segment the NPS curve over time.
pub async fn submit(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    ensure_tables(&pool).await;
    let uid = resolve_user(&pool, &user.privy_did).await?;
    let score = body.get("score").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let touchpoint = body.get("touchpoint").and_then(|v| v.as_str()).unwrap_or("unspecified");
    if !(0..=10).contains(&score) {
        return Ok(Json(serde_json::json!({ "success": false, "error": "score must be 0-10" })));
    }
    sqlx::query("INSERT INTO nps_submissions (user_id, score, touchpoint) VALUES ($1, $2, $3)")
        .bind(uid).bind(score).bind(touchpoint)
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "ok": true } })))
}

/// POST /rescue/submit { reason, score }
///
/// Called by the detractor-rescue screen when a score ≤ 6 user taps one of
/// the pre-canned "why" buttons. The reason is free-form text — the iOS
/// client sends one of the five options, but accepting arbitrary strings
/// keeps the schema future-proof if the list changes.
pub async fn rescue_submit(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    ensure_tables(&pool).await;
    let uid = resolve_user(&pool, &user.privy_did).await?;
    let score = body.get("score").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
    let reason = body.get("reason").and_then(|v| v.as_str()).unwrap_or("unspecified");
    sqlx::query("INSERT INTO rescue_submissions (user_id, score, reason) VALUES ($1, $2, $3)")
        .bind(uid).bind(score).bind(reason)
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "ok": true } })))
}

/// POST /referrals/track { code, channel }
///
/// Called when a promoter copies / shares their referral code. Channel is a
/// short tag ("clipboard", "sms", "share_sheet") so marketing can see which
/// share surface converts best.
pub async fn referral_track(user: AuthUser, State(pool): State<PgPool>, Json(body): Json<serde_json::Value>) -> AppResult<Json<serde_json::Value>> {
    ensure_tables(&pool).await;
    let uid = resolve_user(&pool, &user.privy_did).await?;
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("unknown");
    let channel = body.get("channel").and_then(|v| v.as_str()).unwrap_or("unspecified");
    sqlx::query("INSERT INTO referral_tracks (user_id, code, channel) VALUES ($1, $2, $3)")
        .bind(uid).bind(code).bind(channel)
        .execute(&pool).await?;
    Ok(Json(serde_json::json!({ "success": true, "data": { "ok": true } })))
}
