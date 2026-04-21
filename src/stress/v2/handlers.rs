//! Stress v2 HTTP handlers.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::State, Json};
use sqlx::PgPool;

/// GET /api/v1/stress/sources — real Work / Sleep-debt / Caffeine
/// stress breakdown for the last 7 days.
///
///   work       = avg stress during work hours (10-18) last 7 days
///   sleep_debt = Σ max(0, 8 − total_hours) over last 7 nights, hours
///   caffeine   = morning HR lift (8-10h avg HR minus 30d resting HR)
///                as a 0-100 proxy for caffeine load
///
/// All three are honest derivations — no fake numbers. Returns null
/// per field when upstream data is absent so the UI can render "—".
pub async fn get_sources(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;

    // Work stress — stress during typical work hours.
    let work: Option<f64> = sqlx::query_scalar(
        r#"SELECT AVG(stress)::float8
           FROM hrv
           WHERE user_id=$1
             AND stress IS NOT NULL
             AND timestamp > NOW() - INTERVAL '7 days'
             AND EXTRACT(hour FROM timestamp) BETWEEN 10 AND 18"#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    // Sleep debt — Σ shortfalls below 8 h over the last 7 nights.
    let sleep_debt: Option<f64> = sqlx::query_scalar(
        r#"SELECT COALESCE(SUM(GREATEST(0, 8 - total_hours)), 0)::float8
           FROM sleep_records
           WHERE user_id=$1 AND date > CURRENT_DATE - INTERVAL '7 days'"#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;

    // Caffeine proxy — morning HR lift above the 30-day resting baseline.
    let morning_hr: Option<f64> = sqlx::query_scalar(
        r#"SELECT AVG(bpm)::float8
           FROM heart_rate
           WHERE user_id=$1
             AND timestamp > NOW() - INTERVAL '7 days'
             AND EXTRACT(hour FROM timestamp) BETWEEN 8 AND 10"#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;
    let resting_hr: Option<f64> = sqlx::query_scalar(
        r#"SELECT (PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY bpm))::float8
           FROM heart_rate
           WHERE user_id=$1 AND timestamp > NOW() - INTERVAL '30 days'"#,
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;
    let caffeine = match (morning_hr, resting_hr) {
        (Some(m), Some(r)) if r > 0.0 => Some(((m - r) / 0.3).clamp(0.0, 100.0)),
        _ => None,
    };

    Ok(Json(serde_json::json!({
        "success": true,
        "data": {
            "work": work,
            "sleep_debt_hours": sleep_debt,
            "caffeine": caffeine,
        }
    })))
}

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
    // The `stress_samples_1min` aggregator (when populated) writes
    // the CURRENT snapshot stress score every minute. If the live
    // HRV doesn't change for a while those rows are all identical
    // (seen: 56.3 × 20+ rows for this user → flat 10 identical
    // bars on iOS). Fallback-to-hrv bucketing is always more
    // informative because it reflects real per-sample variance.
    //
    // Heuristic: keep the aggregator path only when it has
    // MEANINGFUL variance (stddev > 3). Otherwise discard and fall
    // back to hrv bucketing so the chart has real movement.
    let aggregator_variance: Option<f64> = if rows.is_empty() {
        None
    } else {
        let mean = rows.iter().map(|r| r.1).sum::<f64>() / rows.len() as f64;
        let variance = rows.iter().map(|r| (r.1 - mean).powi(2)).sum::<f64>() / rows.len() as f64;
        Some(variance.sqrt())
    };
    let use_aggregator = aggregator_variance.map(|sd| sd >= 3.0).unwrap_or(false);
    let points: Vec<serde_json::Value> = if !use_aggregator {
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
        "/stress/v2/intraday user={} pre_agg={} agg_sd={:?} use_agg={} returning {} points",
        user.privy_did, pre_agg_count, aggregator_variance, use_aggregator, points.len()
    );
    Ok(Json(serde_json::json!({"points": points})))
}
