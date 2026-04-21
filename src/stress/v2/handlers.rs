//! Stress v2 HTTP handlers.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::State, Json};
use sqlx::PgPool;

pub async fn get_intraday(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;

    // Prefer the pre-aggregated `stress_samples_1min` table when a
    // backfill worker has populated it. Until that exists we fall
    // back to bucketing the raw `hrv` rows into 15-min windows —
    // iOS was hitting the fallback (local-fabricated ±3-jitter
    // series of the current HRV) because this handler returned
    // `{points: []}` for 24h. The bucketing below reflects real
    // per-sample stress (we store stress alongside rmssd now).
    let rows: Vec<(chrono::DateTime<chrono::Utc>, f64, String, bool)> = sqlx::query_as(
        "SELECT ts, score, level, micro_pulse FROM stress_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let pre_agg_count = rows.len();
    let points: Vec<serde_json::Value> = if rows.is_empty() {
        let buckets: Vec<(chrono::DateTime<chrono::Utc>, f64)> = sqlx::query_as(
            r#"
            SELECT
                date_trunc('minute', timestamp) - (EXTRACT(minute FROM timestamp)::int % 15 || ' minutes')::interval AS bucket,
                AVG(stress)::float8 AS avg_stress
            FROM hrv
            WHERE user_id=$1
              AND timestamp > NOW() - INTERVAL '24 hours'
              AND stress IS NOT NULL
            GROUP BY 1
            ORDER BY 1 ASC
            "#
        )
        .bind(user_id)
        .fetch_all(&pool)
        .await
        .map_err(AppError::from)?;

        buckets.into_iter().map(|(ts, s)| {
            let level = match s as i64 {
                i if i < 25 => "low",
                i if i < 45 => "mild",
                i if i < 65 => "moderate",
                i if i < 85 => "elevated",
                _ => "severe",
            };
            serde_json::json!({"ts": ts, "score": s, "level": level, "micro_pulse": false})
        }).collect()
    } else {
        rows.into_iter().map(|(ts, s, l, p)| {
            serde_json::json!({"ts": ts, "score": s, "level": l, "micro_pulse": p})
        }).collect()
    };

    tracing::info!(
        "/stress/v2/intraday user={} pre_agg={} returning {} points (first: {}, last: {})",
        user.privy_did, pre_agg_count, points.len(),
        points.first().map(|p| p.to_string()).unwrap_or_else(|| "none".into()),
        points.last().map(|p| p.to_string()).unwrap_or_else(|| "none".into())
    );
    Ok(Json(serde_json::json!({"points": points})))
}
