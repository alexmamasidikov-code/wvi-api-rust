//! Emotion v2 baseline locking.
//!
//! Per-context (weekday/time-of-day/activity) baselines are snapshot from
//! the last 14 days of emotion samples once a minimum sample count is met.
//! Reuses Project B's baseline lock semantics — the `locked` flag prevents
//! opportunistic regressions when a user has a bad stretch.

use sqlx::PgPool;
use uuid::Uuid;

pub async fn lock_if_ready(pool: &PgPool, user_id: Uuid) -> sqlx::Result<()> {
    let contexts: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT context_key FROM emotion_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '14 days'",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    for (ctx,) in contexts {
        let stats: Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>, i64)> = sqlx::query_as(
            "SELECT AVG(valence), STDDEV_POP(valence), AVG(arousal), STDDEV_POP(arousal), COUNT(*)
             FROM emotion_samples_1min
             WHERE user_id=$1 AND context_key=$2 AND ts > NOW() - INTERVAL '14 days'",
        )
        .bind(user_id)
        .bind(&ctx)
        .fetch_optional(pool)
        .await?;

        if let Some((Some(vm), vs, Some(am), as_, n)) = stats {
            if n >= 50 {
                let v_std = vs.unwrap_or(0.01).max(0.01);
                let a_std = as_.unwrap_or(0.01).max(0.01);
                sqlx::query(
                    "INSERT INTO user_emotion_baselines
                        (user_id, context_key, v_mean, a_mean, v_std, a_std, locked, last_updated)
                     VALUES ($1, $2, $3, $4, $5, $6, true, NOW())
                     ON CONFLICT (user_id, context_key) DO UPDATE SET
                       v_mean=$3, a_mean=$4, v_std=$5, a_std=$6, locked=true, last_updated=NOW()",
                )
                .bind(user_id)
                .bind(&ctx)
                .bind(vm)
                .bind(am)
                .bind(v_std)
                .bind(a_std)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}
