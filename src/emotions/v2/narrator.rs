//! Emotion v2 narrator — 3 layers: morning forecast, evening journey,
//! contextual per-screen insight. Each is cached in
//! `emotion_ai_narratives` keyed by (user, narrative_type).

use sqlx::PgPool;
use uuid::Uuid;

use crate::ai::cli::{ask_or_fallback, AiEndpointKind};

pub async fn morning_forecast(pool: &PgPool, user_id: Uuid) -> anyhow::Result<String> {
    let metrics = crate::emotions::v2::metrics::compute(pool, user_id).await?;
    let anchors_str = metrics
        .anchors
        .iter()
        .map(|(e, p)| format!("{e} {:.0}%", p * 100.0))
        .collect::<Vec<_>>()
        .join(", ");
    let prompt = format!(
        "Юзер проснулся. Вчерашние якоря эмоций: {anchors_str}. \
         Ответь 2 предложениями на русском — что сегодня в фокусе. \
         Без медицинских диагнозов."
    );
    let text = ask_or_fallback(AiEndpointKind::DailyBrief, &prompt).await;
    upsert_cache(pool, user_id, "morning_forecast", &text).await?;
    Ok(text)
}

pub async fn evening_journey(pool: &PgPool, user_id: Uuid) -> anyhow::Result<String> {
    let samples: Vec<(chrono::DateTime<chrono::Utc>, String)> = sqlx::query_as(
        "SELECT ts, primary_emotion FROM emotion_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '16 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let timeline = samples
        .iter()
        .step_by(30)
        .map(|(t, e)| format!("{} {}", t.format("%H:%M"), e))
        .collect::<Vec<_>>()
        .join(" → ");
    let prompt = format!(
        "Emotional journey today: {timeline}.\n\
         Напиши ~120 слов на русском — путь дня, без медицинских диагнозов."
    );
    let text = ask_or_fallback(AiEndpointKind::EveningReview, &prompt).await;
    upsert_cache(pool, user_id, "evening_journey", &text).await?;
    Ok(text)
}

pub async fn contextual(pool: &PgPool, user_id: Uuid, screen: &str) -> anyhow::Result<String> {
    let cached: Option<(String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
        "SELECT content, generated_at FROM emotion_ai_narratives
         WHERE user_id=$1 AND narrative_type=$2",
    )
    .bind(user_id)
    .bind(screen)
    .fetch_optional(pool)
    .await?;
    if let Some((c, g)) = &cached {
        if chrono::Utc::now() - *g < chrono::Duration::minutes(10) {
            return Ok(c.clone());
        }
    }
    let prompt = format!(
        "User opened screen \"{screen}\". 1 short insight in Russian about the \
         current emotional state. No medical diagnoses."
    );
    let text = ask_or_fallback(AiEndpointKind::Insights, &prompt).await;
    upsert_cache(pool, user_id, screen, &text).await?;
    Ok(text)
}

async fn upsert_cache(
    pool: &PgPool,
    user_id: Uuid,
    key: &str,
    content: &str,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO emotion_ai_narratives (user_id, narrative_type, content, generated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (user_id, narrative_type) DO UPDATE SET
           content=EXCLUDED.content, generated_at=NOW()",
    )
    .bind(user_id)
    .bind(key)
    .bind(content)
    .execute(pool)
    .await?;
    Ok(())
}

/// Morning/evening daily crons, per-user-timezone. Ticks every 5 minutes,
/// fires morning_forecast at 07:00 local and evening_journey at 21:00 local,
/// guarded by `daily_brief_log` so we never double-fire.
pub fn spawn_daily_crons(pool: PgPool) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(tokio::time::Duration::from_secs(300));
        loop {
            tick.tick().await;
            if let Err(e) = run_cycle(&pool).await {
                tracing::error!(?e, "emotion narrator cycle failed");
            }
        }
    });
}

async fn run_cycle(pool: &PgPool) -> sqlx::Result<()> {
    let users: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, COALESCE(s.timezone, 'UTC')
         FROM users u
         LEFT JOIN app_settings s ON s.user_id = u.id",
    )
    .fetch_all(pool)
    .await?;

    for (user_id, tz_str) in users {
        let tz: chrono_tz::Tz = tz_str.parse().unwrap_or(chrono_tz::UTC);
        let now_local = chrono::Utc::now().with_timezone(&tz);

        if crate::narrator_schedule::should_fire_morning(&now_local)
            && crate::narrator_schedule::record_fire(pool, user_id, "morning", &now_local)
                .await
                .unwrap_or(false)
        {
            let _ = morning_forecast(pool, user_id).await;
        }
        if crate::narrator_schedule::should_fire_evening(&now_local)
            && crate::narrator_schedule::record_fire(pool, user_id, "evening", &now_local)
                .await
                .unwrap_or(false)
        {
            let _ = evening_journey(pool, user_id).await;
        }
    }
    Ok(())
}
