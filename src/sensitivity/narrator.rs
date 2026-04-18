//! AI narrator — 4 layers on top of the Claude CLI (`ai::cli`):
//!   * `realtime_critical_takeaway` — one short Russian reading for a fresh signal.
//!   * `daily_morning_brief` — ~2-3 sentences recap of last 24h.
//!   * `evening_pattern_review` — ~120 words pattern review over the day.
//!   * `contextual_insight` — 1-2 sentences insight for a specific screen (cached 10 min).

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::ai::cli::{ask_or_fallback, AiEndpointKind};

pub async fn realtime_critical_takeaway(
    pool: &PgPool,
    signal_id: Uuid,
) -> anyhow::Result<String> {
    let row: (String, String, f64, Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT metric_type, direction, deviation_sigma, bayesian_confidence, rarity_percentile
         FROM signals WHERE id=$1",
    )
    .bind(signal_id)
    .fetch_one(pool)
    .await?;
    let (metric, direction, sigma, bayesian, rarity) = row;

    let prompt = format!(
        "Только что обнаружен сигнал: metric={metric}, direction={direction}, σ={sigma:.1}, \
         bayesian_conf={:.2}, rarity_percentile={:.2}.\n\n\
         2 короткие фразы на русском: что это вероятно означает + конкретное действие.\n\
         Не более 40 слов. Без медицинских диагнозов.",
        bayesian.unwrap_or(0.0),
        rarity.unwrap_or(0.5)
    );
    let text = ask_or_fallback(AiEndpointKind::AnomalyAlert, &prompt).await;
    let cleaned = sanitize(&text);
    sqlx::query("UPDATE signals SET narrative=$1 WHERE id=$2")
        .bind(&cleaned)
        .bind(signal_id)
        .execute(pool)
        .await?;
    Ok(cleaned)
}

pub async fn daily_morning_brief(pool: &PgPool, user_id: Uuid) -> anyhow::Result<String> {
    let signals_24h: Vec<(String, String, f64)> = sqlx::query_as(
        "SELECT metric_type, direction, deviation_sigma FROM signals
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours'
         ORDER BY ts DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let summary = signals_24h
        .iter()
        .map(|(m, d, s)| format!("{m} {d} {s:.1}σ"))
        .collect::<Vec<_>>()
        .join(", ");

    let prompt = format!(
        "Ты персональный health coach. Юзер проснулся.\n\
         Сигналы последних 24h: {summary}.\n\n\
         Сформулируй 2-3 предложения на русском: что произошло и что фокус на сегодня.\n\
         Тёплый, конкретный. Не клише. Не более 80 слов."
    );
    let text = ask_or_fallback(AiEndpointKind::DailyBrief, &prompt).await;
    let cleaned = sanitize(&text);
    upsert_cache(pool, user_id, "morning_brief", &cleaned).await?;
    Ok(cleaned)
}

pub async fn evening_pattern_review(pool: &PgPool, user_id: Uuid) -> anyhow::Result<String> {
    let signals: Vec<(String, String, f64, DateTime<Utc>)> = sqlx::query_as(
        "SELECT metric_type, direction, deviation_sigma, ts FROM signals
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '16 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let composites: Vec<(String,)> = sqlx::query_as(
        "SELECT pair_id FROM composite_signals
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '16 hours'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let timeline = signals
        .iter()
        .map(|(m, d, s, t)| format!("{}: {m} {d} {s:.1}σ", t.format("%H:%M")))
        .collect::<Vec<_>>()
        .join("\n");
    let comp = composites
        .iter()
        .map(|(p,)| p.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let prompt = format!(
        "День закончился. Вот паттерн сигналов:\n\
         {timeline}\n\n\
         Композитные сигналы: {comp}.\n\n\
         Напиши ~120 слов на русском: что происходит в теле/ментально,\n\
         что рекомендуешь на завтра. Без медицинских диагнозов."
    );
    let text = ask_or_fallback(AiEndpointKind::EveningReview, &prompt).await;
    let cleaned = sanitize(&text);
    upsert_cache(pool, user_id, "evening_review", &cleaned).await?;
    Ok(cleaned)
}

pub async fn contextual_insight(
    pool: &PgPool,
    user_id: Uuid,
    screen: &str,
) -> anyhow::Result<String> {
    // 10-minute cache — contextual insights don't need realtime freshness.
    let cached: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT content, generated_at FROM ai_insights_cache
         WHERE user_id=$1 AND screen_key=$2",
    )
    .bind(user_id)
    .bind(screen)
    .fetch_optional(pool)
    .await?;
    if let Some((content, gen_at)) = cached {
        if Utc::now() - gen_at < Duration::minutes(10) {
            return Ok(content);
        }
    }

    let active_signals: Vec<(String, String)> = sqlx::query_as(
        "SELECT metric_type, severity FROM signals
         WHERE user_id=$1 AND NOT ack AND ts > NOW() - INTERVAL '2 hours'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let sigs_str = active_signals
        .iter()
        .map(|(m, s)| format!("{m}[{s}]"))
        .collect::<Vec<_>>()
        .join(", ");

    let prompt = format!(
        "Юзер открыл экран \"{screen}\". Активные сигналы: {sigs_str}.\n\
         Один короткий insight на русском (1-2 предложения) — что обратить внимание на этом экране.\n\
         Не более 30 слов. Без клише."
    );
    let text = ask_or_fallback(AiEndpointKind::Insights, &prompt).await;
    let cleaned = sanitize(&text);
    upsert_cache(pool, user_id, screen, &cleaned).await?;
    Ok(cleaned)
}

fn sanitize(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.len() < 10 || trimmed.len() > 600 {
        return "Анализ недоступен.".to_string();
    }
    let lower = trimmed.to_lowercase();
    for bad in ["у тебя диагноз", "ты болен", "диагностирую"] {
        if lower.contains(bad) {
            return "Анализ недоступен.".to_string();
        }
    }
    trimmed.to_string()
}

async fn upsert_cache(
    pool: &PgPool,
    user_id: Uuid,
    key: &str,
    content: &str,
) -> sqlx::Result<()> {
    let hash = format!("{:x}", md5::compute(content.as_bytes()));
    sqlx::query(
        "INSERT INTO ai_insights_cache (user_id, screen_key, payload_hash, content, generated_at)
         VALUES ($1, $2, $3, $4, NOW())
         ON CONFLICT (user_id, screen_key) DO UPDATE SET
             payload_hash=EXCLUDED.payload_hash,
             content=EXCLUDED.content,
             generated_at=NOW()",
    )
    .bind(user_id)
    .bind(key)
    .bind(&hash)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(())
}

/// Per-user TZ-aware scheduler. Ticks every 5 minutes and fires each user's
/// morning brief at 07:00 and evening review at 21:00 local time, guarded by
/// `daily_brief_log` so we never double-fire inside the same local day.
pub fn spawn_daily_crons(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(300));
        loop {
            tick.tick().await;
            if let Err(e) = run_user_schedule(&pool).await {
                tracing::error!(?e, "sensitivity daily crons cycle failed");
            }
        }
    });
}

async fn run_user_schedule(pool: &PgPool) -> sqlx::Result<()> {
    for (user_id, tz_str) in list_users_with_tz(pool).await? {
        let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
        let now_local = Utc::now().with_timezone(&tz);

        if crate::narrator_schedule::should_fire_morning(&now_local) {
            if crate::narrator_schedule::record_fire(pool, user_id, "morning", &now_local)
                .await
                .unwrap_or(false)
            {
                let _ = daily_morning_brief(pool, user_id).await;
            }
        }
        if crate::narrator_schedule::should_fire_evening(&now_local) {
            if crate::narrator_schedule::record_fire(pool, user_id, "evening", &now_local)
                .await
                .unwrap_or(false)
            {
                let _ = evening_pattern_review(pool, user_id).await;
            }
        }
    }
    Ok(())
}

/// Users + timezone. `timezone` lives in `app_settings`, not `users`, so we
/// LEFT JOIN and fall back to `UTC` for users without saved settings.
async fn list_users_with_tz(pool: &PgPool) -> sqlx::Result<Vec<(Uuid, String)>> {
    sqlx::query_as(
        "SELECT u.id, COALESCE(s.timezone, 'UTC')
         FROM users u
         LEFT JOIN app_settings s ON s.user_id = u.id",
    )
    .fetch_all(pool)
    .await
}
