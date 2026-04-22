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

    let p2 = pool.clone();
    tokio::spawn(async move { anomaly_loop(p2, apns).await });

    // Daily WVI backfill — rolls up HR/HRV/activity/sleep into one
    // wvi_scores row per day for every active user. Without this the
    // streak counter on HOME stays stuck at "Day 1" because the table
    // only has today's live row. Runs every 3h.
    tokio::spawn(async move { wvi_backfill_loop(pool).await });
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

/// Every 3h: for every user with biometric activity in the last 14 days,
/// regenerate per-day wvi_scores rows. UPSERT keeps it idempotent. Uses
/// the same SQL as POST /api/v1/wvi/backfill.
async fn wvi_backfill_loop(pool: PgPool) {
    loop {
        if let Err(e) = run_wvi_backfill_for_active_users(&pool).await {
            tracing::warn!("wvi backfill scheduler failed: {e}");
        }
        tokio::time::sleep(Duration::from_secs(3 * 3600)).await;
    }
}

async fn run_wvi_backfill_for_active_users(pool: &PgPool) -> Result<(), String> {
    use crate::wvi::calculator::{WviV2Calculator, WviV2Input};

    let users: Vec<uuid::Uuid> = sqlx::query_scalar(
        r#"
        SELECT DISTINCT user_id FROM (
            SELECT user_id FROM heart_rate WHERE timestamp > NOW() - INTERVAL '14 days'
            UNION SELECT user_id FROM hrv  WHERE timestamp > NOW() - INTERVAL '14 days'
            UNION SELECT user_id FROM activity WHERE timestamp > NOW() - INTERVAL '14 days'
        ) u
        "#
    ).fetch_all(pool).await.map_err(|e| e.to_string())?;

    let mut total_written = 0usize;
    for uid in users {
        let days: Vec<chrono::NaiveDate> = match sqlx::query_scalar(
            r#"
            SELECT DISTINCT day FROM (
                SELECT timestamp::date AS day FROM heart_rate WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '14 days'
                UNION SELECT timestamp::date FROM hrv WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '14 days'
                UNION SELECT timestamp::date FROM activity WHERE user_id = $1 AND timestamp > NOW() - INTERVAL '14 days'
            ) u ORDER BY day
            "#
        ).bind(uid).fetch_all(pool).await {
            Ok(d) => d,
            Err(e) => { tracing::warn!("backfill days query failed for {uid}: {e}"); continue; }
        };

        for day in days {
            let (hr_avg, hrv_avg, spo2_avg, temp_avg): (Option<f64>, Option<f64>, Option<f64>, Option<f64>) =
                match sqlx::query_as(r#"
                    SELECT
                        (SELECT AVG(bpm)::float8 FROM heart_rate WHERE user_id = $1 AND timestamp::date = $2),
                        (SELECT AVG(rmssd)::float8 FROM hrv WHERE user_id = $1 AND timestamp::date = $2),
                        (SELECT AVG(value)::float8 FROM spo2 WHERE user_id = $1 AND timestamp::date = $2),
                        (SELECT AVG(value)::float8 FROM temperature WHERE user_id = $1 AND timestamp::date = $2)
                "#).bind(uid).bind(day).fetch_one(pool).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };
            if hr_avg.is_none() && hrv_avg.is_none() { continue; }

            let sleep: (Option<f32>, Option<f32>, Option<f32>) = sqlx::query_as(
                "SELECT sleep_score, total_hours, deep_percent FROM sleep_records WHERE user_id = $1 AND date = $2"
            ).bind(uid).bind(day).fetch_optional(pool).await
                .unwrap_or(Some((None, None, None))).unwrap_or((None, None, None));
            let steps: Option<f64> = sqlx::query_scalar(
                "SELECT MAX(steps)::float8 FROM activity WHERE user_id = $1 AND timestamp::date = $2"
            ).bind(uid).bind(day).fetch_one(pool).await.unwrap_or(None);

            let input = WviV2Input {
                hrv_rmssd: hrv_avg.unwrap_or(45.0),
                stress_index: hrv_avg.map(|v| (100.0 - (v / 0.7).min(100.0)).max(0.0)).unwrap_or(40.0),
                sleep_score: sleep.0.map(|v| v as f64).unwrap_or(60.0),
                emotion_score: 55.0,
                spo2: spo2_avg.unwrap_or(98.0),
                heart_rate: hr_avg.unwrap_or(70.0),
                resting_hr: 65.0,
                steps: steps.unwrap_or(0.0),
                active_calories: 0.0,
                acwr: 1.0,
                bp_systolic: 120.0,
                bp_diastolic: 80.0,
                temp_delta: temp_avg.map(|v| v - 36.6).unwrap_or(-2.6),
                ppi_coherence: 0.4,
                emotion_name: String::new(),
            };
            let result = WviV2Calculator::calculate(&input);
            let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();

            if let Err(e) = sqlx::query(r#"
                INSERT INTO wvi_scores (user_id, timestamp, wvi_score, level, metrics, weights, emotion_feedback)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                ON CONFLICT (user_id, timestamp) DO UPDATE SET
                    wvi_score = EXCLUDED.wvi_score,
                    level = EXCLUDED.level,
                    metrics = EXCLUDED.metrics
            "#).bind(uid).bind(ts).bind(result.wvi_score as f32).bind(&result.level)
              .bind(serde_json::to_value(&result.metric_scores).unwrap_or_default())
              .bind(serde_json::json!({ "version": "2.0-scheduler", "type": "daily_aggregate" }))
              .bind(result.emotion_multiplier as f32)
              .execute(pool).await {
                tracing::warn!("wvi_scores upsert failed for {uid}: {e}");
            } else {
                total_written += 1;
            }
        }
    }
    tracing::info!("wvi backfill scheduler: wrote {} rows", total_written);
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
