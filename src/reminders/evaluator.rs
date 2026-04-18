//! Proactive reminders evaluator cron.
//!
//! Runs every 5 min. For each user with master switch enabled and per-reminder
//! conditions met, dispatches an APNs push via `ApnsClient::send_alert`
//! (same pattern as `push::scheduler::scan_anomalies`). Biometric gates read
//! from the intraday tables populated by Project A.

use chrono::{Duration, Timelike, Utc};
use sqlx::PgPool;
use tokio::time::{interval, Duration as TokioDuration};
use uuid::Uuid;

use crate::push::apns::ApnsClient;

pub async fn run(pool: PgPool, apns: ApnsClient) {
    let mut tick = interval(TokioDuration::from_secs(300)); // every 5 min
    loop {
        tick.tick().await;
        if let Err(e) = run_cycle(&pool, &apns).await {
            tracing::error!(?e, "reminders evaluator cycle failed");
        }
    }
}

async fn run_cycle(pool: &PgPool, apns: &ApnsClient) -> anyhow::Result<()> {
    // Users with master switch on + their timezone.
    let users: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, COALESCE(u.timezone, 'UTC')
         FROM users u
         JOIN user_reminder_master m ON m.user_id = u.id
         WHERE m.enabled = true",
    )
    .fetch_all(pool)
    .await?;

    for (user_id, tz_str) in users {
        let settings: Vec<(String, bool, i16, i16, i32, String, Option<chrono::DateTime<Utc>>)> =
            sqlx::query_as(
                "SELECT reminder_type, enabled, start_hour, end_hour, min_interval_min, intensity, last_fired_at
                 FROM user_reminder_settings
                 WHERE user_id = $1 AND enabled = true",
            )
            .bind(user_id)
            .fetch_all(pool)
            .await?;

        for (rtype, _, start, end, interval_min, _, last_fired) in settings {
            let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
            let local_hour = Utc::now().with_timezone(&tz).hour() as i16;
            if local_hour < start || local_hour >= end {
                continue;
            }

            if let Some(last) = last_fired {
                if Utc::now() - last < Duration::minutes(interval_min as i64) {
                    continue;
                }
            }

            let should_fire = match rtype.as_str() {
                "water" => check_water(pool, user_id).await.unwrap_or(false),
                "stand" => check_stand(pool, user_id).await.unwrap_or(false),
                "breathe" => check_breathe(pool, user_id).await.unwrap_or(false),
                "bedtime" => check_bedtime(pool, user_id).await.unwrap_or(false),
                "move" => check_move(pool, user_id, local_hour).await.unwrap_or(false),
                "wvi_drop" => check_wvi_drop(pool, user_id).await.unwrap_or(false),
                _ => false,
            };

            if should_fire {
                if let Err(e) = fire_reminder(pool, apns, user_id, &rtype).await {
                    tracing::warn!(?e, user_id = ?user_id, rtype, "fire_reminder failed");
                }
            }
        }
    }
    Ok(())
}

async fn check_water(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    // Fire if active today (activity_intensity daily mean > 500).
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT value_mean FROM biometrics_daily
         WHERE user_id = $1 AND metric_type = 'activity_intensity' AND day = CURRENT_DATE",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(v,)| v > 500.0).unwrap_or(false))
}

async fn check_stand(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    // Sedentary ≥ 45 min: last 45 rows of activity_intensity all below 10.
    let rows: Vec<(f64,)> = sqlx::query_as(
        "SELECT value FROM biometrics_1min
         WHERE user_id = $1 AND metric_type = 'activity_intensity'
           AND ts > NOW() - INTERVAL '45 minutes'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let sedentary = rows.len() >= 40 && rows.iter().all(|(v,)| *v < 10.0);
    Ok(sedentary)
}

async fn check_breathe(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    let row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT AVG(value) FROM biometrics_1min
         WHERE user_id = $1 AND metric_type = 'stress' AND ts > NOW() - INTERVAL '10 minutes'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let avg = row.and_then(|(v,)| v).unwrap_or(0.0);
    Ok(avg > 70.0)
}

async fn check_bedtime(_pool: &PgPool, _user_id: Uuid) -> sqlx::Result<bool> {
    // Placeholder: bedtime window integration wired once /sleep/optimal-window
    // persists recommendations. Until then rely on the fallback iOS scheduler.
    Ok(false)
}

async fn check_move(pool: &PgPool, user_id: Uuid, local_hour: i16) -> sqlx::Result<bool> {
    if local_hour != 16 {
        return Ok(false);
    }
    let row: Option<(Option<f64>,)> = sqlx::query_as(
        "SELECT SUM(value) FROM biometrics_1min
         WHERE user_id = $1 AND metric_type = 'activity_intensity'
           AND ts::date = CURRENT_DATE",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let total = row.and_then(|(v,)| v).unwrap_or(0.0);
    Ok(total < 300.0)
}

async fn check_wvi_drop(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    let now_val: Option<(f64,)> = sqlx::query_as(
        "SELECT value FROM biometrics_1min
         WHERE user_id = $1 AND metric_type = 'wvi' AND ts > NOW() - INTERVAL '5 minutes'
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let past_val: Option<(f64,)> = sqlx::query_as(
        "SELECT value FROM biometrics_1min
         WHERE user_id = $1 AND metric_type = 'wvi'
           AND ts BETWEEN NOW() - INTERVAL '65 minutes' AND NOW() - INTERVAL '55 minutes'
         ORDER BY ts DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(match (now_val, past_val) {
        (Some((n,)), Some((p,))) => p - n > 10.0,
        _ => false,
    })
}

async fn fire_reminder(
    pool: &PgPool,
    apns: &ApnsClient,
    user_id: Uuid,
    rtype: &str,
) -> anyhow::Result<()> {
    let (title, body_text, deep_link) = template_for(rtype);
    if title.is_empty() {
        return Ok(());
    }

    // Dispatch to every device token for this user. Mirrors the pattern used
    // by `push::scheduler::scan_anomalies`.
    let tokens: Vec<String> =
        sqlx::query_scalar::<_, String>("SELECT token FROM push_tokens WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();

    for token in tokens {
        if let Err(e) = apns.send_alert(&token, title, body_text, Some(deep_link)).await {
            tracing::warn!(%e, user_id = ?user_id, rtype, "apns reminder send failed");
        }
    }

    sqlx::query(
        "UPDATE user_reminder_settings SET last_fired_at = NOW()
         WHERE user_id = $1 AND reminder_type = $2",
    )
    .bind(user_id)
    .bind(rtype)
    .execute(pool)
    .await?;

    Ok(())
}

/// (title, body, deeplink) templates for each reminder type. iOS uses
/// `userInfo["deeplink"]` (see PushNotificationManager) — keep the `deeplink`
/// JSON key so tap-through routing works.
fn template_for(rtype: &str) -> (&'static str, &'static str, &'static str) {
    match rtype {
        "water" => (
            "💧 Выпей воды",
            "Поддержи гидрацию",
            "wellex://reminders/water",
        ),
        "stand" => (
            "🚶 Встань размяться",
            "45 минут сидения — пора двигаться",
            "wellex://reminders/stand",
        ),
        "breathe" => (
            "🧘 2 минуты дыхания?",
            "Пульс напряжённый — помогу выровнять",
            "wellex://mind/breathing",
        ),
        "bedtime" => (
            "🌙 Готовься ко сну",
            "Оптимальное окно приближается",
            "wellex://reminders/bedtime",
        ),
        "move" => (
            "⚡ 10-минутная прогулка",
            "Мало шагов — поднимем WVI",
            "wellex://reminders/move",
        ),
        "wvi_drop" => (
            "📉 WVI падает",
            "Что-то меняется — проверь",
            "wellex://body/wvi",
        ),
        _ => ("", "", ""),
    }
}

pub fn spawn(pool: PgPool, apns: ApnsClient) {
    tokio::spawn(async move { run(pool, apns).await });
}
