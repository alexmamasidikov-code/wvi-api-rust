//! Baseline engine — per-context rolling personal baselines.
//!
//! * Pre-onboarding (user age <14d): collect samples only.
//! * Lock at day 14 via `lock_baselines`: computes mean/std/p10/p90 per context.
//! * Post-lock: EWMA drift with α=0.01 so baselines slowly track genuine change
//!   without absorbing outliers.

use crate::sensitivity::types::{Baseline, ContextKey, DetectorState};
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn load(
    pool: &PgPool,
    user_id: Uuid,
    metric: &str,
    ctx: &ContextKey,
) -> sqlx::Result<Option<Baseline>> {
    sqlx::query_as(
        "SELECT mean, std, p10, p90, sample_count, locked
         FROM user_baselines WHERE user_id=$1 AND metric_type=$2 AND context_key=$3",
    )
    .bind(user_id)
    .bind(metric)
    .bind(ctx.as_str())
    .fetch_optional(pool)
    .await
}

pub async fn update(
    pool: &PgPool,
    user_id: Uuid,
    metric: &str,
    ctx: &ContextKey,
    sample: f64,
) -> sqlx::Result<()> {
    let existing = load(pool, user_id, metric, ctx).await?;
    match existing {
        Some(b) if b.locked => {
            // EWMA drift post-lock: α=0.01 — very slow adaptation.
            let alpha = 0.01;
            let new_mean = b.mean + alpha * (sample - b.mean);
            let new_var = b.std.powi(2) + alpha * ((sample - b.mean).powi(2) - b.std.powi(2));
            let new_std = new_var.max(0.01).sqrt();
            sqlx::query(
                "UPDATE user_baselines SET mean=$1, std=$2,
                    sample_count=sample_count+1, last_updated=NOW()
                 WHERE user_id=$3 AND metric_type=$4 AND context_key=$5",
            )
            .bind(new_mean)
            .bind(new_std)
            .bind(user_id)
            .bind(metric)
            .bind(ctx.as_str())
            .execute(pool)
            .await?;
        }
        Some(_) => {
            // Pre-lock: just bump sample_count; `lock_baselines` computes real
            // stats at day 14 from biometrics_1min.
            sqlx::query(
                "UPDATE user_baselines SET sample_count=sample_count+1, last_updated=NOW()
                 WHERE user_id=$1 AND metric_type=$2 AND context_key=$3",
            )
            .bind(user_id)
            .bind(metric)
            .bind(ctx.as_str())
            .execute(pool)
            .await?;
        }
        None => {
            // First sample: insert placeholder (locked=false) so later samples
            // increment the counter.
            sqlx::query(
                "INSERT INTO user_baselines
                    (user_id, metric_type, context_key, mean, std, p10, p90, sample_count, locked)
                 VALUES ($1, $2, $3, $4, 1.0, $4, $4, 1, false)",
            )
            .bind(user_id)
            .bind(metric)
            .bind(ctx.as_str())
            .bind(sample)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn is_past_onboarding(pool: &PgPool, user_id: Uuid) -> sqlx::Result<bool> {
    let row: Option<(DateTime<Utc>,)> =
        sqlx::query_as("SELECT created_at FROM users WHERE id=$1")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(t,)| Utc::now() - t > Duration::days(14)).unwrap_or(false))
}

/// Compute locked stats for every (metric, context) row this user has placeholder
/// data for. Idempotent — can be rerun; skips rows with too few samples.
pub async fn lock_baselines(pool: &PgPool, user_id: Uuid) -> sqlx::Result<()> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT metric_type, context_key FROM user_baselines WHERE user_id=$1 AND NOT locked",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    for (metric, ctx_key) in rows {
        let stats: Option<(Option<f64>, Option<f64>, Option<f64>, Option<f64>, i64)> = sqlx::query_as(
            "SELECT
                AVG(value), STDDEV_POP(value),
                PERCENTILE_CONT(0.10) WITHIN GROUP (ORDER BY value),
                PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY value),
                COUNT(*)
             FROM biometrics_1min
             WHERE user_id=$1 AND metric_type=$2 AND ts > NOW() - INTERVAL '14 days'",
        )
        .bind(user_id)
        .bind(&metric)
        .fetch_optional(pool)
        .await?;

        if let Some((Some(m), Some(s), Some(p10), Some(p90), count)) = stats {
            if count >= 10 {
                sqlx::query(
                    "UPDATE user_baselines SET mean=$1, std=$2, p10=$3, p90=$4,
                        locked=true, last_updated=NOW()
                     WHERE user_id=$5 AND metric_type=$6 AND context_key=$7",
                )
                .bind(m)
                .bind(s.max(0.01))
                .bind(p10)
                .bind(p90)
                .bind(user_id)
                .bind(&metric)
                .bind(&ctx_key)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}

pub async fn get_detector_state(
    pool: &PgPool,
    user_id: Uuid,
    metric: &str,
) -> sqlx::Result<DetectorState> {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT state FROM detector_state WHERE user_id=$1 AND metric_type=$2")
            .bind(user_id)
            .bind(metric)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .and_then(|(v,)| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub async fn save_detector_state(
    pool: &PgPool,
    user_id: Uuid,
    metric: &str,
    state: &DetectorState,
) -> sqlx::Result<()> {
    let json = serde_json::to_value(state).unwrap();
    sqlx::query(
        "INSERT INTO detector_state (user_id, metric_type, state, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (user_id, metric_type) DO UPDATE SET
             state=EXCLUDED.state, updated_at=NOW()",
    )
    .bind(user_id)
    .bind(metric)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}
