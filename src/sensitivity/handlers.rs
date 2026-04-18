//! HTTP handlers for the sensitivity module.
//!   GET  /api/v1/signals          — list detected signals (filters: period/severity/metric)
//!   PUT  /api/v1/signals/{id}/ack — acknowledge one
//!   GET  /api/v1/insights/contextual?screen=… — 10-min cached AI blurb
//!   GET  /api/v1/baselines?metric=…&context=… — current baseline row

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct SignalsQuery {
    pub period: Option<String>,
    pub severity: Option<String>,
    pub metric: Option<String>,
}

pub async fn get_signals(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<SignalsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did)
        .await
        .map_err(AppError::from)?;
    let start = match q.period.as_deref() {
        Some("7d") => Utc::now() - Duration::days(7),
        _ => Utc::now() - Duration::hours(24),
    };
    let severity_filter = q.severity.unwrap_or_else(|| "all".into());
    let metric_filter = q.metric.unwrap_or_else(|| "all".into());

    let rows: Vec<(
        Uuid,
        DateTime<Utc>,
        String,
        String,
        f64,
        String,
        String,
        Option<String>,
        Option<f64>,
        Option<f64>,
        bool,
    )> = sqlx::query_as(
        "SELECT id, ts, metric_type, context_key, deviation_sigma, direction, severity,
                narrative, bayesian_confidence, rarity_percentile, ack
         FROM signals
         WHERE user_id=$1 AND ts >= $2
           AND ($3='all' OR severity=$3)
           AND ($4='all' OR metric_type=$4)
         ORDER BY ts DESC LIMIT 200",
    )
    .bind(user_id)
    .bind(start)
    .bind(&severity_filter)
    .bind(&metric_filter)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(id, ts, m, ctx, sigma, dir, sev, nar, bay, rar, ack)| {
            serde_json::json!({
                "id": id,
                "ts": ts,
                "metric_type": m,
                "context_key": ctx,
                "deviation_sigma": sigma,
                "direction": dir,
                "severity": sev,
                "narrative": nar,
                "bayesian_confidence": bay,
                "rarity_percentile": rar,
                "ack": ack,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn ack_signal(
    user: AuthUser,
    State(pool): State<PgPool>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did)
        .await
        .map_err(AppError::from)?;
    sqlx::query("UPDATE signals SET ack=true WHERE id=$1 AND user_id=$2")
        .bind(id)
        .bind(user_id)
        .execute(&pool)
        .await
        .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "acked": true })))
}

#[derive(Deserialize)]
pub struct InsightQuery {
    pub screen: String,
}

pub async fn get_contextual(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<InsightQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did)
        .await
        .map_err(AppError::from)?;
    let content = crate::sensitivity::narrator::contextual_insight(&pool, user_id, &q.screen)
        .await
        .unwrap_or_else(|_| "Анализ недоступен.".into());
    Ok(Json(serde_json::json!({
        "content": content,
        "generated_at": Utc::now(),
    })))
}

#[derive(Deserialize)]
pub struct BaselineQuery {
    pub metric: String,
    pub context: Option<String>,
}

pub async fn get_baseline(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<BaselineQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did)
        .await
        .map_err(AppError::from)?;
    let ctx = q.context.unwrap_or_else(|| {
        crate::sensitivity::types::ContextKey::from_ts(
            Utc::now(),
            crate::sensitivity::types::ActivityState::Resting,
        )
        .as_str()
    });
    let row: Option<(f64, f64, f64, f64, i32, bool)> = sqlx::query_as(
        "SELECT mean, std, p10, p90, sample_count, locked
         FROM user_baselines WHERE user_id=$1 AND metric_type=$2 AND context_key=$3",
    )
    .bind(user_id)
    .bind(&q.metric)
    .bind(&ctx)
    .fetch_optional(&pool)
    .await
    .map_err(AppError::from)?;

    match row {
        Some((m, s, p10, p90, count, locked)) => Ok(Json(serde_json::json!({
            "mean": m,
            "std": s,
            "p10": p10,
            "p90": p90,
            "sample_count": count,
            "locked": locked,
        }))),
        None => Ok(Json(serde_json::json!({ "locked": false, "sample_count": 0 }))),
    }
}

/// Unread signal count for the last 7 days. Used by the HOME notification
/// dot — wire via `GET /api/v1/signals/unread-count`.
pub async fn get_unread_count(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did)
        .await
        .map_err(AppError::from)?;
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM signals WHERE user_id=$1 AND NOT ack AND ts > NOW() - INTERVAL '7 days'"
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .map_err(AppError::from)?;
    Ok(Json(serde_json::json!({ "count": row.0 })))
}
