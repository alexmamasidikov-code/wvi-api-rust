use chrono::{DateTime, Duration, DurationRound, Timelike, Utc};
use sqlx::PgPool;
use tokio::time::{interval, Duration as TokioDuration};
use uuid::Uuid;

pub async fn run_worker(pool: PgPool) {
    let mut tick = interval(TokioDuration::from_secs(60)); // check every minute
    loop {
        tick.tick().await;
        let now = Utc::now();
        if let Err(e) = run_5min_tick(&pool, now).await {
            tracing::error!(?e, "intraday 5-min tick failed");
        }
        // hourly rollup at :00 (within 1-minute tolerance)
        if now.minute() == 0 {
            if let Err(e) = run_hourly_rollup(&pool, now).await {
                tracing::error!(?e, "intraday hourly rollup failed");
            }
        }
    }
}

async fn run_5min_tick(pool: &PgPool, now: DateTime<Utc>) -> sqlx::Result<()> {
    // Only run at 5-min boundary
    if now.minute() % 5 != 0 {
        return Ok(());
    }
    let bucket_end = now.duration_trunc(Duration::minutes(5)).unwrap();
    let bucket_start = bucket_end - Duration::minutes(5);

    // Aggregate raw buckets
    sqlx::query(
        "INSERT INTO biometrics_5min (user_id, bucket_ts, metric_type,
              value_mean, value_min, value_max, sample_count, formula_version)
         SELECT user_id, $1::TIMESTAMPTZ AS bucket_ts, metric_type,
                AVG(value), MIN(value), MAX(value), COUNT(*), MAX(formula_version)
         FROM biometrics_1min
         WHERE ts >= $2 AND ts < $3
         GROUP BY user_id, metric_type
         ON CONFLICT (user_id, bucket_ts, metric_type) DO NOTHING",
    )
    .bind(bucket_start) // $1 — bucket_ts column value
    .bind(bucket_start) // $2 — lower bound (inclusive)
    .bind(bucket_end)   // $3 — upper bound (exclusive)
    .execute(pool)
    .await?;

    // Project B — feed the freshly-aggregated bucket into the sensitivity
    // detection + composite pipelines. Each metric evaluation is independent,
    // so we fire-and-forget them concurrently per user/metric to keep the
    // 5-min tick latency bounded. Errors are logged but never bubble up
    // (downsampling must always succeed even if detection has a hiccup).
    let users_metrics: Vec<(Uuid, String, f64)> = sqlx::query_as(
        "SELECT user_id, metric_type, value_mean
         FROM biometrics_5min WHERE bucket_ts=$1",
    )
    .bind(bucket_start)
    .fetch_all(pool)
    .await?;

    for (user_id, metric, value) in users_metrics {
        let ts = bucket_start;
        // Activity state: placeholder Resting until Project C ingests
        // activity_intensity into the sensitivity context directly.
        let activity = crate::sensitivity::types::ActivityState::Resting;
        let pool_c = pool.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::sensitivity::detection::evaluate_bucket(
                &pool_c, user_id, &metric, ts, value, activity,
            )
            .await
            {
                tracing::warn!(?e, ?user_id, metric = %metric, "sensitivity::evaluate_bucket failed");
            }
            if let Err(e) =
                crate::sensitivity::composite::evaluate(&pool_c, user_id).await
            {
                tracing::warn!(?e, ?user_id, "sensitivity::composite evaluate failed");
            }
        });
    }

    Ok(())
}

async fn run_hourly_rollup(pool: &PgPool, now: DateTime<Utc>) -> sqlx::Result<()> {
    // Aggregate previous completed day (run at 00:00; UTC for MVP)
    if now.hour() != 0 {
        return Ok(());
    }
    let day = (now - Duration::days(1)).date_naive();

    sqlx::query(
        "INSERT INTO biometrics_daily (user_id, day, metric_type,
             value_mean, value_min, value_max, value_p10, value_p90, volatility_sd)
         SELECT user_id, $1::DATE, metric_type,
                AVG(value_mean), MIN(value_min), MAX(value_max),
                PERCENTILE_CONT(0.10) WITHIN GROUP (ORDER BY value_mean),
                PERCENTILE_CONT(0.90) WITHIN GROUP (ORDER BY value_mean),
                STDDEV_POP(value_mean)
         FROM biometrics_5min
         WHERE bucket_ts >= $2 AND bucket_ts < $3
         GROUP BY user_id, metric_type
         ON CONFLICT (user_id, day, metric_type) DO NOTHING",
    )
    .bind(day)
    .bind(day.and_hms_opt(0, 0, 0).unwrap().and_utc())
    .bind((day + chrono::Days::new(1)).and_hms_opt(0, 0, 0).unwrap().and_utc())
    .execute(pool)
    .await?;
    Ok(())
}

pub fn spawn(pool: PgPool) {
    tokio::spawn(async move {
        run_worker(pool).await;
    });
}
