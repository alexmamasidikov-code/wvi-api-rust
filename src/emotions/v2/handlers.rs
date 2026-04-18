//! Emotion v2 HTTP handlers.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use sqlx::PgPool;

pub async fn get_intraday(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let rows: Vec<(
        chrono::DateTime<chrono::Utc>,
        f64, f64, f64,
        String, f64,
        String, f64,
        String, f64,
    )> = sqlx::query_as(
        "SELECT ts, valence, arousal, confidence,
                primary_emotion, primary_intensity,
                secondary_emotion, secondary_intensity,
                tertiary_emotion, tertiary_intensity
         FROM emotion_samples_1min
         WHERE user_id=$1 AND ts > NOW() - INTERVAL '24 hours'
         ORDER BY ts ASC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;

    let points: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(ts, v, a, c, p, pi, s, si, t, ti)| {
            serde_json::json!({
                "ts": ts,
                "valence": v,
                "arousal": a,
                "confidence": c,
                "primary": {"emotion": p, "intensity": pi},
                "secondary": {"emotion": s, "intensity": si},
                "tertiary": {"emotion": t, "intensity": ti},
            })
        })
        .collect();
    Ok(Json(serde_json::json!({"points": points})))
}

pub async fn get_metrics(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<crate::emotions::v2::metrics::EmotionMetrics>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let m = crate::emotions::v2::metrics::compute(&pool, user_id)
        .await
        .map_err(|e| AppError::Internal(format!("{e}")))?;
    Ok(Json(m))
}

#[derive(Deserialize)]
pub struct NarrativeQuery {
    pub r#type: String,
}

pub async fn get_narrative(
    user: AuthUser,
    State(pool): State<PgPool>,
    Query(q): Query<NarrativeQuery>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let content = match q.r#type.as_str() {
        "morning" => crate::emotions::v2::narrator::morning_forecast(&pool, user_id).await,
        "evening" => crate::emotions::v2::narrator::evening_journey(&pool, user_id).await,
        other => crate::emotions::v2::narrator::contextual(&pool, user_id, other).await,
    }
    .unwrap_or_else(|_| "Анализ недоступен.".into());
    Ok(Json(serde_json::json!({"content": content})))
}

pub async fn get_triggers(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let rows: Vec<(
        chrono::DateTime<chrono::Utc>,
        uuid::Uuid,
        f64,
        String,
        String,
        f64,
    )> = sqlx::query_as(
        "SELECT shift_ts, event_id, correlation_p, shift_from_region, shift_to_region, magnitude
         FROM emotion_triggers WHERE user_id=$1 AND shift_ts > NOW() - INTERVAL '7 days'
         ORDER BY shift_ts DESC",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await?;

    let items: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|(ts, ev, p, from, to, mag)| {
            serde_json::json!({
                "ts": ts, "event_id": ev, "correlation_p": p,
                "from": from, "to": to, "magnitude": mag
            })
        })
        .collect();
    Ok(Json(serde_json::json!({"items": items})))
}
