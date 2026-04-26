//! WVI v3 HTTP handlers.

use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::State, Json};
use chrono::Utc;
use serde::Deserialize;
use sqlx::PgPool;

pub async fn get_current(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<crate::wvi::v3::aggregator::WviV3Result>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;

    // Refuse to compute when there are no recent biometric inputs at all.
    // The v3 aggregator fills missing components with a 50.0 placeholder
    // (per-component baseline queries are still TODO), so calling this
    // for a disconnected account collapsed to a believable "score = 51"
    // hero. Clients (HeroWVIView / WVIViewModel) treat a 404 as
    // "no data" and fall back to the empty state.
    const FRESHNESS_HOURS: i64 = 6;
    let cutoff = Utc::now() - chrono::Duration::hours(FRESHNESS_HOURS);
    let newest: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        r#"SELECT MAX(latest_ts) FROM (
              SELECT MAX(timestamp) latest_ts FROM heart_rate  WHERE user_id = $1
              UNION ALL
              SELECT MAX(timestamp)            FROM hrv         WHERE user_id = $1
              UNION ALL
              SELECT MAX(timestamp)            FROM spo2        WHERE user_id = $1
              UNION ALL
              SELECT MAX(timestamp)            FROM temperature WHERE user_id = $1
              UNION ALL
              SELECT MAX(timestamp)            FROM activity    WHERE user_id = $1
           ) t"#
    ).bind(user_id).fetch_one(&pool).await.unwrap_or(None);
    if newest.map_or(true, |t| t < cutoff) {
        return Err(AppError::NotFound("no_recent_biometrics".into()));
    }

    let result = crate::wvi::v3::aggregator::compute_wvi_v3(&pool, user_id, Utc::now())
        .await
        .map_err(|e| AppError::Internal(format!("{e}")))?;
    Ok(Json(result))
}

pub async fn get_forecast(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<crate::wvi::v3::forecast::WviForecast>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let current = crate::wvi::v3::aggregator::compute_wvi_v3(&pool, user_id, Utc::now())
        .await
        .ok()
        .map(|r| r.current)
        .unwrap_or(50.0);
    let f = crate::wvi::v3::forecast::forecast_wvi(&pool, user_id, Utc::now(), current)
        .await
        .map_err(|e| AppError::Internal(format!("{e}")))?;
    Ok(Json(f))
}

#[derive(Deserialize)]
pub struct ProfileBody {
    pub profile: Option<String>,
    pub display_mode: Option<String>,
}

pub async fn put_profile(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<ProfileBody>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    // Upsert: preserve existing values when caller only sets one field.
    sqlx::query(
        "INSERT INTO user_wvi_profile (user_id, profile, display_mode, updated_at)
         VALUES ($1, COALESCE($2, 'balanced'), COALESCE($3, 'rich'), NOW())
         ON CONFLICT (user_id) DO UPDATE SET
           profile=COALESCE($2, user_wvi_profile.profile),
           display_mode=COALESCE($3, user_wvi_profile.display_mode),
           updated_at=NOW()",
    )
    .bind(user_id)
    .bind(body.profile.as_deref())
    .bind(body.display_mode.as_deref())
    .execute(&pool)
    .await?;
    Ok(Json(serde_json::json!({"saved": true})))
}

pub async fn get_profile_suggest(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let (suggested, confidence, reasoning) =
        crate::wvi::v3::profile_classifier::suggest(&pool, user_id)
            .await
            .map_err(|e| AppError::Internal(format!("{e}")))?;
    Ok(Json(
        serde_json::json!({"suggested": suggested, "confidence": confidence, "reasoning": reasoning}),
    ))
}
