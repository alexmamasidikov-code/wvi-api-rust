//! Background push scheduler.
//!
//! Two jobs:
//!   - Morning brief at 07:00 local-ish (07:00 UTC for now; localization
//!     layer can be added later). Reads the prewarmed `daily_brief` from
//!     AppCache so no extra CLI spawn is needed.
//!   - Anomaly watch: every 5 min, scan recent WVI scores and push if the
//!     current score dropped >15 points vs the 24h baseline.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::time::Duration;

use crate::ai::cli::AiEndpointKind;
use crate::ai::handlers::cache_key;
use crate::cache::AppCache;

use super::apns::ApnsClient;

pub fn spawn_scheduler(pool: PgPool, cache: AppCache, apns: ApnsClient) {
    let p = pool.clone();
    let c = cache.clone();
    let a = apns.clone();
    tokio::spawn(async move { morning_brief_loop(p, c, a).await });

    tokio::spawn(async move { anomaly_loop(pool, apns).await });
}

async fn morning_brief_loop(pool: PgPool, cache: AppCache, apns: ApnsClient) {
    loop {
        let now: DateTime<Utc> = Utc::now();
        let next = next_utc_seven(now);
        let wait = next.signed_duration_since(now).to_std().unwrap_or(Duration::from_secs(60));
        tracing::info!("push/morning: next fire in {:?}", wait);
        tokio::time::sleep(wait).await;
        if let Err(e) = send_morning_briefs(&pool, &cache, &apns).await {
            tracing::warn!("morning brief push failed: {e}");
        }
    }
}

fn next_utc_seven(now: DateTime<Utc>) -> DateTime<Utc> {
    let today_seven = now.date_naive().and_hms_opt(7, 0, 0).unwrap().and_utc();
    if today_seven > now { today_seven } else { today_seven + chrono::Duration::days(1) }
}

async fn send_morning_briefs(pool: &PgPool, cache: &AppCache, apns: &ApnsClient) -> Result<(), String> {
    let users: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        r#"
        SELECT DISTINCT u.id, u.privy_did
        FROM users u
        JOIN push_tokens t ON t.user_id = u.id
        WHERE t.last_seen_at > NOW() - INTERVAL '30 days'
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for (user_id, privy_did) in users {
        let key = cache_key(&privy_did, AiEndpointKind::DailyBrief);
        let body = cache.get_ai(&key).await
            .unwrap_or_else(|| AiEndpointKind::DailyBrief.fallback_text().to_string());
        // APNs body cap ~256 chars for alert — trim gracefully.
        let trimmed = trim_for_push(&body, 200);
        for token in tokens_for(pool, user_id).await {
            if let Err(e) = apns.send_alert(&token, "Good morning", &trimmed, Some("wellex://dashboard/ai-coach")).await {
                tracing::warn!("apns send failed for user {user_id}: {e}");
            }
        }
    }
    Ok(())
}

async fn anomaly_loop(pool: PgPool, apns: ApnsClient) {
    loop {
        tokio::time::sleep(Duration::from_secs(300)).await;
        if let Err(e) = scan_anomalies(&pool, &apns).await {
            tracing::warn!("anomaly scan failed: {e}");
        }
    }
}

async fn scan_anomalies(pool: &PgPool, apns: &ApnsClient) -> Result<(), String> {
    // Simple heuristic: latest WVI score < (24h median - 15). One alert per
    // user per 6 hours (dedupe via last_seen_at window).
    let rows: Vec<(uuid::Uuid, f64, f64)> = sqlx::query_as(
        r#"
        WITH latest AS (
            SELECT DISTINCT ON (user_id) user_id, score, calculated_at
            FROM wvi_scores
            WHERE calculated_at > NOW() - INTERVAL '30 minutes'
            ORDER BY user_id, calculated_at DESC
        ),
        baseline AS (
            SELECT user_id, percentile_cont(0.5) WITHIN GROUP (ORDER BY score) AS med
            FROM wvi_scores
            WHERE calculated_at > NOW() - INTERVAL '24 hours'
            GROUP BY user_id
        )
        SELECT l.user_id, l.score::float8, b.med::float8
        FROM latest l JOIN baseline b USING(user_id)
        WHERE (b.med - l.score) > 15
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for (user_id, score, med) in rows {
        let delta = (med - score).round() as i32;
        let body = format!("WVI just dropped {delta} points to {:.0}. Open the app for a quick recovery plan.", score);
        for token in tokens_for(pool, user_id).await {
            if let Err(e) = apns.send_alert(&token, "WVI alert", &body, Some("wellex://dashboard/ai-coach")).await {
                tracing::warn!("apns anomaly push failed for user {user_id}: {e}");
            }
        }
    }
    Ok(())
}

async fn tokens_for(pool: &PgPool, user_id: uuid::Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT token FROM push_tokens WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

fn trim_for_push(text: &str, max: usize) -> String {
    let clean = text.replace('\n', " ");
    if clean.chars().count() <= max { return clean; }
    let mut out: String = clean.chars().take(max - 1).collect();
    out.push('…');
    out
}
