use crate::auth::middleware::AuthUser;
use crate::error::{AppError, AppResult};
use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ReminderSetting {
    pub reminder_type: String,
    pub enabled: bool,
    pub start_hour: i16,
    pub end_hour: i16,
    pub min_interval_min: i32,
    pub intensity: String,
    pub last_fired_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize)]
pub struct ReminderSettings {
    pub master_enabled: bool,
    pub settings: Vec<ReminderSetting>,
}

#[derive(Deserialize)]
pub struct PutBody {
    pub master_enabled: bool,
    pub settings: Vec<ReminderSetting>,
}

pub async fn get_settings(
    user: AuthUser,
    State(pool): State<PgPool>,
) -> AppResult<Json<ReminderSettings>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    let master: Option<(bool,)> = sqlx::query_as(
        "SELECT enabled FROM user_reminder_master WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&pool)
    .await
    .map_err(AppError::from)?;
    let settings: Vec<ReminderSetting> = sqlx::query_as(
        "SELECT reminder_type, enabled, start_hour, end_hour, min_interval_min, intensity, last_fired_at
         FROM user_reminder_settings WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&pool)
    .await
    .map_err(AppError::from)?;
    Ok(Json(ReminderSettings {
        master_enabled: master.map(|(e,)| e).unwrap_or(true),
        settings,
    }))
}

pub async fn put_settings(
    user: AuthUser,
    State(pool): State<PgPool>,
    Json(body): Json<PutBody>,
) -> AppResult<Json<serde_json::Value>> {
    let user_id = crate::users::resolve_user_id(&pool, &user.privy_did).await?;
    sqlx::query(
        "INSERT INTO user_reminder_master (user_id, enabled, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (user_id) DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = NOW()",
    )
    .bind(user_id)
    .bind(body.master_enabled)
    .execute(&pool)
    .await
    .map_err(AppError::from)?;

    for s in body.settings {
        sqlx::query(
            "INSERT INTO user_reminder_settings (user_id, reminder_type, enabled,
                                                 start_hour, end_hour, min_interval_min, intensity)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (user_id, reminder_type) DO UPDATE SET
               enabled          = EXCLUDED.enabled,
               start_hour       = EXCLUDED.start_hour,
               end_hour         = EXCLUDED.end_hour,
               min_interval_min = EXCLUDED.min_interval_min,
               intensity        = EXCLUDED.intensity",
        )
        .bind(user_id)
        .bind(&s.reminder_type)
        .bind(s.enabled)
        .bind(s.start_hour)
        .bind(s.end_hour)
        .bind(s.min_interval_min)
        .bind(&s.intensity)
        .execute(&pool)
        .await
        .map_err(AppError::from)?;
    }
    Ok(Json(serde_json::json!({ "saved": true })))
}
