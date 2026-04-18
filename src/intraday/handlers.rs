use crate::auth::middleware::AuthUser;
use crate::intraday::{lttb, types::*};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use sqlx::PgPool;

#[derive(Deserialize)]
pub struct IntradayQuery {
    pub metric: String,
    pub period: String, // "24h"|"7d"|"30d"|"90d"|"365d"
    #[allow(dead_code)]
    pub metrics: Option<String>,
    pub compare: Option<String>,
    pub include_events: Option<bool>,
    pub resolution: Option<String>,
}

pub async fn get_intraday(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<IntradayQuery>,
) -> Result<Json<IntradayResponse>, StatusCode> {
    let end = Utc::now();
    let start = match q.period.as_str() {
        "24h" => end - Duration::hours(24),
        "7d" => end - Duration::days(7),
        "30d" => end - Duration::days(30),
        "90d" => end - Duration::days(90),
        "365d" => end - Duration::days(365),
        _ => return Err(StatusCode::BAD_REQUEST),
    };

    // Read 1min for 24h, 5min otherwise
    let (points_raw, resolution) = if q.period == "24h" {
        let rows: Vec<(chrono::DateTime<Utc>, f64)> = sqlx::query_as(
            "SELECT ts, value FROM biometrics_1min
             WHERE user_id=(SELECT id FROM users WHERE privy_did=$1)
               AND metric_type=$2 AND ts BETWEEN $3 AND $4
             ORDER BY ts ASC",
        )
        .bind(&user.privy_did)
        .bind(&q.metric)
        .bind(start)
        .bind(end)
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let pts: Vec<ChartPoint> = rows
            .into_iter()
            .map(|(ts, v)| ChartPoint { ts, value: v, min: None, max: None })
            .collect();
        (pts, "1min".to_string())
    } else {
        let rows: Vec<(chrono::DateTime<Utc>, f64, Option<f64>, Option<f64>)> = sqlx::query_as(
            "SELECT bucket_ts, value_mean, value_min, value_max FROM biometrics_5min
             WHERE user_id=(SELECT id FROM users WHERE privy_did=$1)
               AND metric_type=$2 AND bucket_ts BETWEEN $3 AND $4
             ORDER BY bucket_ts ASC",
        )
        .bind(&user.privy_did)
        .bind(&q.metric)
        .bind(start)
        .bind(end)
        .fetch_all(&pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let pts: Vec<ChartPoint> = rows
            .into_iter()
            .map(|(ts, m, mn, mx)| ChartPoint { ts, value: m, min: mn, max: mx })
            .collect();
        (pts, "5min".to_string())
    };

    // LTTB downsample
    let target = match q.resolution.as_deref() {
        Some("sparkline") => 40,
        _ => 288,
    };
    let points = lttb::downsample(&points_raw, target);

    // Events
    let events = if q.include_events.unwrap_or(false) {
        let rows: Vec<(uuid::Uuid, chrono::DateTime<Utc>, String, serde_json::Value)> =
            sqlx::query_as(
                "SELECT id, ts, event_type, meta FROM intraday_events
             WHERE user_id=(SELECT id FROM users WHERE privy_did=$1)
               AND ts BETWEEN $2 AND $3
             ORDER BY ts ASC",
            )
            .bind(&user.privy_did)
            .bind(start)
            .bind(end)
            .fetch_all(&pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        rows.into_iter()
            .map(|(id, ts, event_type, meta)| ChartEvent { id, ts, event_type, meta })
            .collect()
    } else {
        vec![]
    };

    // Compare overlay (yesterday / last_week)
    let compare_points = match q.compare.as_deref() {
        Some("yesterday") => Some(
            fetch_compare(
                &pool,
                &user.privy_did,
                &q.metric,
                start - Duration::days(1),
                end - Duration::days(1),
            )
            .await?,
        ),
        Some("last_week") => Some(
            fetch_compare(
                &pool,
                &user.privy_did,
                &q.metric,
                start - Duration::days(7),
                end - Duration::days(7),
            )
            .await?,
        ),
        _ => None,
    };

    // Backfill in progress?
    let (backfill_in_progress, backfill_progress): (bool, f64) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM backfill_jobs WHERE metric_type=$1 AND status='running'),
                COALESCE((SELECT progress_ratio FROM backfill_jobs WHERE metric_type=$1 AND status='running' LIMIT 1), 1.0)",
    )
    .bind(&q.metric)
    .fetch_one(&pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Formula version
    let version: (i32,) = sqlx::query_as(
        "SELECT version FROM formula_versions WHERE metric_type=$1 ORDER BY deployed_at DESC LIMIT 1",
    )
    .bind(&q.metric)
    .fetch_one(&pool)
    .await
    .unwrap_or((1,));

    Ok(Json(IntradayResponse {
        metric: q.metric,
        period: q.period,
        resolution,
        points,
        events,
        compare_points,
        formula_version: version.0,
        backfill_in_progress,
        backfill_progress,
    }))
}

async fn fetch_compare(
    pool: &PgPool,
    privy_did: &str,
    metric: &str,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
) -> Result<Vec<ChartPoint>, StatusCode> {
    let rows: Vec<(chrono::DateTime<Utc>, f64)> = sqlx::query_as(
        "SELECT bucket_ts, value_mean FROM biometrics_5min
         WHERE user_id=(SELECT id FROM users WHERE privy_did=$1)
           AND metric_type=$2 AND bucket_ts BETWEEN $3 AND $4
         ORDER BY bucket_ts ASC",
    )
    .bind(privy_did)
    .bind(metric)
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let pts: Vec<ChartPoint> = rows
        .into_iter()
        .map(|(ts, v)| ChartPoint { ts, value: v, min: None, max: None })
        .collect();
    Ok(lttb::downsample(&pts, 288))
}

#[derive(Deserialize)]
pub struct BackfillRequest {
    pub metric: String,
    pub new_version: i32,
    pub range_start: chrono::DateTime<Utc>,
    pub range_end: chrono::DateTime<Utc>,
}

pub async fn post_backfill(
    _user: AuthUser, // admin check TODO
    State(pool): State<PgPool>,
    Json(body): Json<BackfillRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id = crate::intraday::backfill::start_backfill(
        &pool,
        body.metric,
        body.new_version,
        body.range_start,
        body.range_end,
    )
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "job_id": id })))
}
