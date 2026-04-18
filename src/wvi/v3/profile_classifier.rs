//! WVI v3 — auto-classify the best-fitting profile for a user based on
//! 30-day activity + stress + WVI volatility patterns.

use sqlx::PgPool;
use uuid::Uuid;

pub async fn suggest(pool: &PgPool, user_id: Uuid) -> anyhow::Result<(String, f64, String)> {
    let stats: Option<(Option<f64>, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT AVG(value) FILTER (WHERE metric_type='activity_intensity') as act,
                AVG(value) FILTER (WHERE metric_type='stress') as stress,
                STDDEV_POP(value) FILTER (WHERE metric_type='wvi') as vol
         FROM biometrics_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '30 days'",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let (act, stress, vol) = stats
        .map(|(a, s, v)| (a.unwrap_or(0.0), s.unwrap_or(50.0), v.unwrap_or(5.0)))
        .unwrap_or((0.0, 50.0, 5.0));

    let (profile, reasoning) = if act > 30.0 {
        ("athlete", "Высокая активность >30 мин/день")
    } else if stress > 70.0 {
        ("stressed", "Средний стресс >70")
    } else if vol > 10.0 {
        ("recovery", "Высокая волатильность WVI — фокус на восстановлении")
    } else {
        ("office", "Стандартный профиль")
    };

    Ok((profile.to_string(), 0.75, reasoning.to_string()))
}
